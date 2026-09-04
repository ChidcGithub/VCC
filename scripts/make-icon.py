# VCC 应用图标：黑玻璃圆角方块 + 白色闪电 + 顶部微光（贴合黑白体系）
from PIL import Image, ImageDraw, ImageFilter
import os

S = 1024
OUT = r"D:\My things\Learn\高二\VCC\src-tauri\icons"

im = Image.new('RGBA', (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(im)
R = 224  # 圆角（≈22%，接近 squircle）
d.rounded_rectangle([0, 0, S-1, S-1], radius=R, fill=(13, 13, 16, 255))

# 顶部微光：白渐变裁进圆角
mask = Image.new('L', (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, S-1, S-1], radius=R, fill=255)
grad = Image.new('L', (1, S))
for y in range(S):
    v = max(0.0, 1.0 - y / (S * 0.5))
    grad.putpixel((0, y), int(26 * (v ** 1.5)))
grad = grad.resize((S, S))
sheen = Image.new('RGBA', (S, S), (255, 255, 255, 255))
im.paste(sheen, (0, 0), Image.composite(grad, Image.new('L', (S, S), 0), mask))

# 闪电（微发光：blur 白色副本 + 锐利主体）
bolt = [(592, 128), (322, 578), (498, 578), (432, 896), (748, 434), (560, 434), (668, 128)]
glow = Image.new('RGBA', (S, S), (0, 0, 0, 0))
ImageDraw.Draw(glow).polygon(bolt, fill=(255, 255, 255, 210))
glow = glow.filter(ImageFilter.GaussianBlur(26))
im.alpha_composite(glow)
d = ImageDraw.Draw(im)
d.polygon(bolt, fill=(255, 255, 255, 255))

os.makedirs(OUT, exist_ok=True)
# ico 多尺寸
im.save(os.path.join(OUT, 'icon.ico'), sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
# png 系列
for name, size in [('32x32.png', 32), ('128x128.png', 128), ('128x128@2x.png', 256)]:
    im.resize((size, size), Image.LANCZOS).save(os.path.join(OUT, name))
print('icons written to', OUT)
