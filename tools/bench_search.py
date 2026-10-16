"""Search speed on this computer: SemSearch against Windows Search — the numbers behind the chart in the READMEs.

Asks the running SemSearch (the port is found by /api/ping, the token is read from the data folder) and the Windows
Search index, the same one Explorer uses (OLE DB from PowerShell, Search.CollatorDSO). Timed is what a person waits for:
from sending the query to having every result row.
- by name: a part of the file name, the first 200 files. SemSearch: the name engine over its pipe (what the backend
  asks), and for reference the whole /api/search?tab=name, which also runs the search by meaning. Windows: CONTAINS(System.FileName, '"word*"') — word-prefix matching like
  the Explorer search box, the fastest form the index has (LIKE '%word%' is several times slower).
- by content: a phrase. SemSearch: /api/search?tab=all, by meaning. Windows: FREETEXT — by words, with word forms.
- by name without an index (--walk): one pass over the whole drive with dir /s, which is what Explorer does outside
  indexed folders. Slow and heavy on the disk, so it runs once, for the first name query only.
Every query gets one warm-up and RUNS timed runs; the median of each query, then the median of the queries, is reported.
Writes tools/readme/speed.js (the data of tools/readme/speed.html) and prints a summary; make_readme_images.py speed
draws the chart from it. SemSearch should be idle (not indexing) while this runs.

Run: .venv\\Scripts\\python.exe tools\\bench_search.py [--walk]
"""
import json
import os
import statistics
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

RUNS = 10
NAME_QUERIES = ["readme", "license", "setup", "config", "report"]
CONTENT_QUERIES = ["договор аренды квартиры", "инструкция по установке", "счёт за электричество", "фото с отпуска на море",
                   "как настроить роутер"]
OUT = Path(__file__).resolve().parent / "readme" / "speed.js"
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))  # engine_client.py

WINDOWS_PS = r"""
$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [Console]::OutputEncoding = [Text.Encoding]::UTF8
$job = [Console]::In.ReadToEnd() | ConvertFrom-Json
$c = New-Object -ComObject ADODB.Connection
$c.CommandTimeout = 0
$c.Open("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';")
function Run($sql) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $rs = $c.Execute($sql); $n = 0
  while (-not $rs.EOF) { $null = $rs.Fields.Item(0).Value; $n++; $rs.MoveNext() }
  $sw.Stop(); return @($sw.Elapsed.TotalMilliseconds, $n)
}
$out = @()
foreach ($q in $job.queries) {
  $sql = $job.template.Replace('{q}', $q.Replace("'", "''"))
  $null = Run $sql
  $ms = @(); $n = 0
  for ($i = 0; $i -lt $job.runs; $i++) { $r = Run $sql; $ms += $r[0]; $n = $r[1] }
  $out += @{ q = $q; ms = $ms; n = $n }
}
ConvertTo-Json -InputObject @($out) -Depth 5 -Compress
"""


def data_dir():
    d = os.environ.get("SEMSEARCH_DATA_DIR")
    return Path(d) if d and os.path.isabs(d) else Path(os.environ["LOCALAPPDATA"]) / "SemSearch"


def find_port():
    for port in range(8765, 8786):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/ping", timeout=1) as r:
                if r.status == 200:
                    return port
        except OSError:
            continue
    sys.exit("SemSearch is not running (nothing answers /api/ping on 8765–8785)")


def semsearch(port, token, queries, tab):
    def ask(q):
        req = urllib.request.Request(f"http://127.0.0.1:{port}/api/search?" + urllib.parse.urlencode({"q": q, "tab": tab}),
                                     headers={"X-SemSearch-Token": token})
        t = time.perf_counter()
        with urllib.request.urlopen(req, timeout=30) as r:
            body = json.loads(r.read())
        return (time.perf_counter() - t) * 1000, len(body.get("items", []))
    out = []
    for q in queries:
        ask(q)
        runs = [ask(q) for _ in range(RUNS)]
        out.append({"q": q, "ms": [m for m, _ in runs], "n": runs[-1][1]})
    return out


def engine(queries):
    from engine_client import EngineClient

    c = EngineClient()

    def ask(q):
        t = time.perf_counter()
        r = c.request("search", q=q, mode="auto", limit=200, files_only=True)
        return (time.perf_counter() - t) * 1000, len(r.get("items", []))
    out = []
    for q in queries:
        ask(q)
        runs = [ask(q) for _ in range(RUNS)]
        out.append({"q": q, "ms": [m for m, _ in runs], "n": runs[-1][1]})
    c.close()
    return out


def windows(queries, template):
    job = json.dumps({"queries": queries, "template": template, "runs": RUNS}, ensure_ascii=False)
    r = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", WINDOWS_PS], input=job,
                       capture_output=True, text=True, encoding="utf-8")
    if r.returncode:
        sys.exit("Windows Search query failed: " + r.stderr.strip()[-400:])
    return json.loads(r.stdout)


def windows_scope():
    """What Windows Search indexes here: the folders of its crawl-scope rules that are included (registry)."""
    ps = ("Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\Windows Search\CrawlScopeManager\Windows\SystemIndex\WorkingSetRules' |"
          " ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.Include -eq 1 -and $_.URL -like 'file:*' } |"
          " ForEach-Object { $_.URL } | ConvertTo-Json -Compress")
    r = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, encoding="utf-8")
    return json.loads(r.stdout or "[]")


def walk(query):
    t = time.perf_counter()
    r = subprocess.run(f'dir "C:\\*{query}*" /s /b /a-d', shell=True, capture_output=True)
    return {"q": query, "ms": [(time.perf_counter() - t) * 1000], "n": r.stdout.count(b"\n")}


def median_of(results):
    return statistics.median(statistics.median(r["ms"]) for r in results)


def machine():
    ps = ("$c = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim();"
          "$g = (Get-CimInstance Win32_VideoController | Where-Object { $_.Name -match 'NVIDIA|AMD|Radeon|Arc' } |"
          " Select-Object -First 1).Name;"
          "$d = (Get-PhysicalDisk | Where-Object { $_.DeviceId -eq ((Get-Partition -DriveLetter C).DiskNumber) }).MediaType;"
          "@{cpu=$c; gpu=$g; disk=\"$d\"} | ConvertTo-Json -Compress")
    r = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True)
    return json.loads(r.stdout)


def main():
    token = (data_dir() / "token").read_text(encoding="utf-8").strip()
    port = find_port()
    with urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{port}/api/status",
                                                       headers={"X-SemSearch-Token": token})) as r:
        status = json.loads(r.read())
    if status.get("state") != "idle":
        print("warning: SemSearch is", status.get("state"), "— the numbers will be worse than at rest", file=sys.stderr)
    win_names = windows(NAME_QUERIES, "SELECT TOP 200 System.ItemPathDisplay FROM SYSTEMINDEX WHERE "
                                      "CONTAINS(System.FileName, '\"{q}*\"')")
    win_text = windows(CONTENT_QUERIES, "SELECT TOP 200 System.ItemPathDisplay FROM SYSTEMINDEX WHERE FREETEXT('{q}')")
    data = {
        "date": time.strftime("%Y-%m-%d"), "runs": RUNS, "machine": machine(), "device": status.get("device"),
        "semsearch_names": status.get("names", {}).get("count"),
        "semsearch_chunks": sum(k.get("done", 0) for k in status.get("kinds", {}).values()),
        "windows_scope": windows_scope(),
        "name": {"semsearch": engine(NAME_QUERIES), "semsearch_window": semsearch(port, token, NAME_QUERIES, "name"),
                 "windows": win_names},
        "content": {"semsearch": semsearch(port, token, CONTENT_QUERIES, "all"), "windows": win_text},
    }
    if "--walk" in sys.argv:
        data["name"]["walk"] = [walk(NAME_QUERIES[0])]
    data["median"] = {g: {k: round(median_of(v), 1) for k, v in data[g].items()} for g in ("name", "content")}
    OUT.write_text("window.SPEED = " + json.dumps(data, ensure_ascii=False, indent=1) + ";\n", encoding="utf-8")
    print(json.dumps({k: data[k] for k in ("machine", "device", "semsearch_names", "semsearch_chunks", "windows_scope", "median")},
                     ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
