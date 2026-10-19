#!/usr/bin/env python3
"""Regenerate our original two-pane icon; Python standard library only."""
from pathlib import Path
import struct
import zlib

DEST = Path(__file__).resolve().parents[2] / "assets/icons"
SHAPES = [
    (24,24,464,464,100,(27,48,71)),
    (68,108,176,296,22,(52,152,219)), (268,108,176,296,22,(106,200,237)),
    (68,108,176,58,22,(223,243,251)), (268,108,176,58,22,(223,243,251)),
    (91,194,130,26,8,(235,248,253)), (91,244,96,18,8,(188,225,247)),
    (91,290,112,18,8,(188,225,247)), (291,194,130,26,8,(255,191,78)),
    (291,244,96,18,8,(235,248,253)), (291,290,112,18,8,(235,248,253)),
]


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def png(size):
    pixels = bytearray(size * size * 4)
    for x,y,w,h,radius,color in SHAPES:
        scale = size / 512
        for row in range(max(0,int(y*scale)), min(size,int((y+h)*scale)+1)):
            for col in range(max(0,int(x*scale)), min(size,int((x+w)*scale)+1)):
                coverage = 0
                for ox,oy in ((.25,.25),(.75,.25),(.25,.75),(.75,.75)):
                    px,py = (col+ox)/scale, (row+oy)/scale
                    cx,cy = min(max(px,x+radius),x+w-radius), min(max(py,y+radius),y+h-radius)
                    coverage += (px-cx)**2 + (py-cy)**2 <= radius**2
                if coverage:
                    offset = (row*size+col)*4
                    alpha = coverage/4
                    old_alpha = pixels[offset+3]/255
                    new_alpha = alpha + old_alpha*(1-alpha)
                    for channel in range(3):
                        pixels[offset+channel] = round((color[channel]*alpha + pixels[offset+channel]*old_alpha*(1-alpha))/new_alpha)
                    pixels[offset+3] = round(new_alpha*255)
    rows = b"".join(b"\0" + pixels[row*size*4:(row+1)*size*4] for row in range(size))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR",struct.pack(">IIBBBBB",size,size,8,6,0,0,0)) + chunk(b"IDAT",zlib.compress(rows,9)) + chunk(b"IEND",b"")


def main():
    DEST.mkdir(parents=True,exist_ok=True)
    svg = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">']
    for x,y,w,h,r,c in SHAPES:
        svg.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="#{c[0]:02x}{c[1]:02x}{c[2]:02x}"/>')
    (DEST/"rust-commander.svg").write_text("\n".join(svg+["</svg>\n"]))
    images = {s:png(s) for s in (16,32,48,64,128,256,512,1024)}
    (DEST/"rust-commander.png").write_bytes(images[512])
    sizes=(16,32,48,64,128,256)
    offset=6+16*len(sizes)
    table=b""
    for s in sizes:
        table+=struct.pack("<BBBBHHII",s%256,s%256,0,0,1,32,len(images[s]),offset)
        offset+=len(images[s])
    (DEST/"rust-commander.ico").write_bytes(struct.pack("<HHH",0,1,len(sizes))+table+b"".join(images[s] for s in sizes))
    icns=b""
    for kind,s in ((b"ic07",128),(b"ic08",256),(b"ic09",512),(b"ic10",1024)):
        icns+=kind+struct.pack(">I",len(images[s])+8)+images[s]
    (DEST/"rust-commander.icns").write_bytes(b"icns"+struct.pack(">I",len(icns)+8)+icns)


if __name__ == "__main__":
    main()
