from PIL import Image, ImageOps, ImageDraw
from pathlib import Path
import sys
root=Path('design')
reference=Image.open(root/'reading-first-reference.png').convert('RGB')
actual=Image.open(root/sys.argv[1]).convert('RGB')
assert reference.size==actual.size, (reference.size,actual.size)
canvas=Image.new('RGB',(reference.width*2,reference.height+32),'white')
canvas.paste(reference,(0,32));canvas.paste(actual,(reference.width,32))
draw=ImageDraw.Draw(canvas)
draw.text((12,10),'SELECTED REFERENCE',fill='black');draw.text((reference.width+12,10),'NATIVE IMPLEMENTATION',fill='black')
canvas.save(root/'comparison-full.png')
for name,box in [('navigation',(0,0,410,620)),('article',(390,185,1235,810)),('toolbar',(310,0,1488,200))]:
    a=reference.crop(box); b=actual.crop(box)
    out=Image.new('RGB',(a.width*2,a.height),'white');out.paste(a,(0,0));out.paste(b,(a.width,0));out.save(root/f'comparison-{name}.png')
