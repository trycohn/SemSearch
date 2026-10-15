"""CUDA Graphs для текстовой башни EmbeddingGemma 2: каждый слой — один запуск графа вместо ~50 ядер.

Зачем. Фоновая индексация идёт маленькими порциями с синхронизацией после каждого слоя (throttle.py), иначе
кадры игр и рабочего стола ждут видеокарту. На таких порциях процессор не успевает подавать ядра по одному,
и внутри каждого всплеска видеокарта полвремени простаивает. Граф слоя подаётся одним вызовом — всплеск той же
длины делает в 1,3–2 раза больше работы. Поиск тоже быстрее: запрос — 24 запуска вместо ~1200 ядер.

Как. Форма входа записывается в граф, поэтому длина дополняется до корзины (bucket_len), а порция — копиями
последней строки (bucket_bs). Входы всех графов лежат в общих буферах (одни адреса для всех форм), временные
тензоры — в общем пуле памяти: графы запускаются строго по одному, под замком модели (core.Embedder.lock).
Новая форма сначала считается обычным путём (с паузами throttle), затем записывается. Изображения и аудио
проходят через ту же текстовую башню (их токены уже вставлены в inputs_embeds) — им графы тоже достаются.
При любой ошибке графы выключаются до перезапуска, и всё считается обычным путём.
"""
import logging
from collections import OrderedDict

import torch
import torch.nn.functional as F
from transformers.masking_utils import create_bidirectional_mask, create_bidirectional_sliding_window_mask
from transformers.modeling_outputs import BaseModelOutput

log = logging.getLogger("cudagraphs")

MAX_TOKENS = 8192  # строк × длина; больше — обычным путём
MAX_LEN = 2048
MAX_SHAPES = 128  # записанных форм (по графу на слой); самая давняя выбрасывается
BATCHES = (1, 2, 3, 4, 6, 8, 12, 16, 24, 32)
MASKS = {"full_attention": create_bidirectional_mask, "sliding_attention": create_bidirectional_sliding_window_mask}


def bucket_len(n):
    """Длина, до которой дополняется порция: кратно 32 токенам до 512, 64 — до 1024, дальше 128."""
    step = 32 if n <= 512 else 64 if n <= 1024 else 128
    return -(-n // step) * step


def bucket_bs(n):
    return next((b for b in BATCHES if b >= n), n)


class LayerGraphs:
    """Подменяет forward текстовой модели (EmbeddingGemma2TextModel) запуском графов по слоям."""

    def __init__(self, lm, throttle):
        self.lm, self.th, cfg = lm, throttle, lm.config
        self.types = list(cfg.layer_types)
        self.hidden = cfg.hidden_size
        self.per_layer = cfg.hidden_size_per_layer_input
        ple = lm.ple
        if len(lm.layers) != len(self.types) or ple.per_layer_model_projection.weight.shape[0] != \
                self.per_layer * len(self.types) or set(self.types) - set(MASKS):
            raise ValueError("модель устроена не так, как ожидалось")
        self.orig = lm.forward
        self.graphs = OrderedDict()  # (строк, длина) -> [граф на слой]
        self.buf = self.pool = self.stream = None
        self.off = False
        self.ooms = 0  # нехватка видеопамяти — не повод отказываться от графов сразу: игра могла занять её на время
        lm.forward = self.forward

    def uninstall(self):
        self.reset()
        self.lm.__dict__.pop("forward", None)

    def reset(self):
        """Отдать видеопамять (пауза ради игры): графы, их пул и буферы создаются заново при следующем расчёте."""
        self.graphs.clear()
        self.buf = self.pool = self.stream = None

    def forward(self, input_ids=None, attention_mask=None, position_ids=None, inputs_embeds=None, **kw):
        e, am = inputs_embeds, attention_mask
        if not (self.off or input_ids is not None or position_ids is not None or e is None or am is None
                or am.dim() != 2 or not e.is_cuda or torch.is_grad_enabled()
                or kw.get("output_attentions") or kw.get("output_hidden_states")):
            bs, n = am.shape
            rows, length = bucket_bs(bs), bucket_len(n)
            if rows * length <= MAX_TOKENS and length <= MAX_LEN:
                try:
                    return self._run(e, am, bs, n, rows, length)
                except Exception as ex:
                    oom = isinstance(ex, torch.cuda.OutOfMemoryError)
                    self.ooms += oom
                    self.off = not oom or self.ooms >= 3  # при нехватке памяти ещё две попытки
                    log.warning("CUDA Graphs %s (%s: %s) — %s обычным путём",
                                "выключены" if self.off else "не поместились", type(ex).__name__, str(ex)[:300],
                                "дальше" if self.off else "эту порцию")
                    self.reset()
        return self.orig(input_ids=input_ids, attention_mask=attention_mask, position_ids=position_ids,
                         inputs_embeds=inputs_embeds, **kw)

    def _views(self, rows, length, rope):
        """Входы графа формы (rows, length) — начала общих буферов, у всех форм одни и те же адреса."""
        if self.buf is None:
            dev, dt = rope[self.types[0]][0].device, rope[self.types[0]][0].dtype
            self.buf = {k: torch.empty(MAX_TOKENS * self.hidden, device=dev, dtype=dt) for k in ("emb", "h")}
            for t, (c, _) in rope.items():
                self.buf["mask", t] = torch.empty(MAX_TOKENS * MAX_LEN, device=dev, dtype=torch.bool)
                for k in ("cos", "sin"):
                    self.buf[k, t] = torch.empty(MAX_LEN * c.shape[-1], device=dev, dtype=dt)
            self.pool, self.stream = torch.cuda.graph_pool_handle(), torch.cuda.Stream(dev)
        n, b = rows * length, self.buf
        v = {k: b[k][:n * self.hidden].view(rows, length, self.hidden) for k in ("emb", "h")}
        for t, (c, _) in rope.items():
            v["mask", t] = b["mask", t][:n * length].view(rows, 1, length, length)
            for k in ("cos", "sin"):
                v[k, t] = b[k, t][:length * c.shape[-1]].view(1, length, c.shape[-1])
        return v

    def _step(self, v, i):
        """Слой i: его срез per-layer-входа (PLE) + сам слой; результат — на место скрытого состояния."""
        lm, t = self.lm, self.types[i]
        p = self.per_layer
        w = lm.ple.per_layer_model_projection.weight[i * p:(i + 1) * p]
        ple = lm.ple.per_layer_projection_norm(F.linear(v["emb"], w) * lm.ple.per_layer_model_projection_scale)
        out = lm.layers[i].forward(v["h"], ple, attention_mask=v["mask", t],
                                   position_embeddings=(v["cos", t], v["sin", t]))
        v["h"].copy_(out)

    def _run(self, e, am, bs, n, rows, length):
        lm = self.lm
        if rows > bs:  # недостающие строки — копии последней, их результат отбрасывается
            e = torch.cat([e, e[-1:].expand(rows - bs, -1, -1)])
            am = torch.cat([am, am[-1:].expand(rows - bs, -1)])
        if length > n:
            e, am = F.pad(e, (0, 0, 0, length - n)), F.pad(am, (0, length - n))
        pos = torch.arange(length, device=e.device).unsqueeze(0)
        rope = {t: lm.rotary_emb(e, pos, t) for t in dict.fromkeys(self.types)}
        v = self._views(rows, length, rope)
        v["emb"].copy_(e)
        v["h"].copy_(e)
        for t, (c, s) in rope.items():
            m = MASKS[t](config=lm.config, inputs_embeds=e, attention_mask=am)
            if m is None:
                v["mask", t].fill_(True)
            elif m.dtype != torch.bool:
                raise TypeError(f"маска {m.dtype}, ожидалась bool")
            else:
                v["mask", t].copy_(m.expand(rows, 1, length, length))
            v["cos", t].copy_(c)
            v["sin", t].copy_(s)
        key = (rows, length)
        graphs = self.graphs.get(key)
        if graphs is None:  # форма впервые: обычным путём (с паузами throttle), потом запись
            for i in range(len(self.types)):
                self._step(v, i)
                self.th.layer_done()
            self._capture(key, v)
        else:
            self.graphs.move_to_end(key)
            for g in graphs:
                g.replay()
                self.th.layer_done()
        return BaseModelOutput(last_hidden_state=lm.embedding_projection(lm.norm(v["h"][:bs, :n])))

    def _capture(self, key, v):
        """Запись ничего не считает, поэтому всплеска не даёт; синхронизация во время записи запрещена —
        точки throttle на это время выключены. thread_local: поиск в соседнем потоке (VecStore) не мешает записи."""
        s = self.stream
        s.wait_stream(torch.cuda.current_stream())
        graphs = []
        with self.th.suspended(), torch.cuda.stream(s):
            for i in range(len(self.types)):
                g = torch.cuda.CUDAGraph()
                g.capture_begin(pool=self.pool, capture_error_mode="thread_local")
                try:
                    self._step(v, i)
                finally:
                    g.capture_end()
                graphs.append(g)
        torch.cuda.current_stream().wait_stream(s)
        self.graphs[key] = graphs
        if len(self.graphs) > MAX_SHAPES:
            self.graphs.popitem(last=False)

    def check(self):
        """Сверка с обычным путём на пробной порции разной длины (есть и дополнение, и маски). -> худший косинус
        по токенам; второй проход идёт уже через записанные графы."""
        lm, gen = self.lm, torch.Generator().manual_seed(0)
        lens = (37, 90, 64)
        ids = torch.randint(1000, 200000, (len(lens), max(lens)), generator=gen)
        am = (torch.arange(max(lens)) < torch.tensor(lens)[:, None]).long()
        dev = lm.embedding_projection.weight.device
        ids, am = ids.to(dev), am.to(dev)
        with torch.inference_mode():
            e = lm.embed_tokens(ids)
            ref = self.orig(inputs_embeds=e, attention_mask=am).last_hidden_state
            worst = 1.0
            for _ in range(2):
                out = self.forward(inputs_embeds=e, attention_mask=am).last_hidden_state
                ok = am.bool()
                worst = min(worst, F.cosine_similarity(out[ok].float(), ref[ok].float(), dim=-1).min().item())
        return worst


def install(model, throttle):
    """-> LayerGraphs или None (модель не та, или сверка с обычным путём не прошла — причина в журнале)."""
    lm = next((m for m in model.modules() if type(m).__name__ == "EmbeddingGemma2TextModel"), None)
    if lm is None:
        log.info("CUDA Graphs: текстовой модели EmbeddingGemma 2 не нашлось — обычным путём")
        return None
    g = None
    try:
        g = LayerGraphs(lm, throttle)
        worst = g.check()
        if g.off or worst < 0.999:
            raise ValueError(f"расходится с обычным путём (косинус {worst:.5f})")
    except Exception as ex:
        log.warning("CUDA Graphs не включены (%s: %s) — текст считается обычным путём", type(ex).__name__,
                    str(ex)[:300])
        if g is not None:
            g.uninstall()
        return None
    log.info("CUDA Graphs по слоям включены (сверка с обычным путём: косинус %.5f)", worst)
    return g
