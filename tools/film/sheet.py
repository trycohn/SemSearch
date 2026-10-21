"""Contact sheet: python sheet.py <dir with PNGs> <out.png> [cols] [thumb width]"""
import os
import sys

from PIL import Image, ImageDraw


def main():
    src, out = sys.argv[1], sys.argv[2]
    cols = int(sys.argv[3]) if len(sys.argv) > 3 else 4
    tw = int(sys.argv[4]) if len(sys.argv) > 4 else 640
    files = sorted(f for f in os.listdir(src) if f.endswith('.png'))
    th = tw * 9 // 16
    rows = (len(files) + cols - 1) // cols
    sheet = Image.new('RGB', (cols * (tw + 8) + 8, rows * (th + 30) + 8), '#888')
    d = ImageDraw.Draw(sheet)
    for k, f in enumerate(files):
        im = Image.open(os.path.join(src, f)).convert('RGB').resize((tw, th), Image.LANCZOS)
        x, y = 8 + (k % cols) * (tw + 8), 8 + (k // cols) * (th + 30)
        sheet.paste(im, (x, y))
        d.text((x + 4, y + th + 6), f, fill='white')
    sheet.save(out)


if __name__ == '__main__':
    main()
