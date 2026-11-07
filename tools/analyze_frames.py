"""items.id=720 (THROWAWAY): is there a horizontal "line shift" in CEF's own output?
Usage: tools/analyze_frames.py <dir with f0000.pgm ...>   (frames from SPIKE720_DUMP_FRAMES, b-sw-below, pane 0, list scrolled)
Finds the 140 px scroll-list box in the first frame, then for every frame measures the left edge of each text line inside it.
A shift would show as left edges that differ within a frame or change between frames. Pure Python, no numpy.
"""
import sys, glob, os, statistics as st
def load(p):
    b=open(p,'rb').read()
    # P5\nW H\n255\n
    parts=b.split(b'\n',3); w,h=map(int,parts[1].split()); return w,h,parts[3]
def box(w,h,d):
    # horizontal border rows of the list: many pixels around 153 (#999)
    rows=[]
    for y in range(h):
        row=d[y*w:(y+1)*w]; n=sum(1 for v in row if 140<=v<=165)
        if n>w*0.6: rows.append(y)
    return rows
def lines(w,h,d,y0,y1,x0,x1):
    """left edge (first dark px) of each text band inside the box"""
    bands=[];cur=None
    for y in range(y0,y1):
        row=d[y*w+x0:y*w+x1]; dark=[i for i,v in enumerate(row) if v<110]
        if dark:
            if cur is None: cur=[y,y,min(dark),max(dark)]
            else: cur[1]=y; cur[2]=min(cur[2],min(dark)); cur[3]=max(cur[3],max(dark))
        elif cur: bands.append(cur); cur=None
    if cur: bands.append(cur)
    return [(b[0],b[1],b[2]+x0,b[3]+x0) for b in bands if b[1]-b[0]>=4]
if __name__=='__main__':
    d0=sys.argv[1]; fs=sorted(glob.glob(d0+'/f*.pgm'))
    w,h,d=load(fs[0]); br=box(w,h,d)
    top,bot=br[0],br[-1]
    cols=[x for x in range(w) if sum(1 for y in range(top+1,bot) if 140<=d[y*w+x]<=165)>(bot-top)*0.7]
    x0,x1=cols[0]+2,cols[-1]-18
    res=[]; moved=0; prev=None
    for p in fs:
        w,h,d=load(p); L=lines(w,h,d,top+2,bot-1,x0,x1); res.append(L)
        sig=tuple((b[0],b[1]) for b in L)
        if prev is not None and sig!=prev: moved+=1
        prev=sig
    e=lambda L:[b[2] for b in L]
    sp=[(max(e(L))-min(e(L))) for L in res if L]
    print('frames',len(fs),'| list box rows',top,bot,'cols',cols[0],cols[-1],'| frames where the lines moved vs previous:',moved)
    print('within-frame left-edge spread: max=%d, frames with spread>0: %d'%(max(sp),sum(1 for v in sp if v)))
    print('distinct median left edges:',sorted(set(st.median(e(L)) for L in res if L)))
