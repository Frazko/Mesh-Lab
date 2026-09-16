from pathlib import Path
import hashlib
# Independent tiny encoder for this fixed vector, not the Rust codec.
def head(m,n):
    for k,t in [(0,24),(1,256),(2,65536),(4,2**32),(8,2**64)]:
        if n<t:return bytes([(m<<5)|n]) if k==0 else bytes([(m<<5)|{1:24,2:25,4:26,8:27}[k]])+n.to_bytes(k,'big')
def c(v):
    if isinstance(v,int):return head(0,v)
    if isinstance(v,bytes):return head(2,len(v))+v
    if isinstance(v,str):return head(3,len(v.encode()))+v.encode()
    return head(4,len(v))+b''.join(c(i) for i in v)
h=c([1,1,bytes([7])*32,bytes([1])*32,24,1,'mesh.lab.text',1,1,100,4,[[bytes([2])*32,bytes([8])*32]],1025,bytes([9])*32])
context=hashlib.sha256(b'MeshLab/EnvelopeContext/v1\0'+h).digest()
r=c([1,1,bytes([7])*32,1,context,bytes([6])*32,bytes([1])*32,24,bytes([2])*32,25])
p=Path('vectors/protocol');(p/'header-v1.hex').write_text(h.hex()+'\n');(p/'context-id-v1.hex').write_text(context.hex()+'\n');(p/'receipt-body-v1.hex').write_text(r.hex()+'\n')
