"""items.id=720 (THROWAWAY): CPU of the whole spike720 process tree by role, idle animation vs scripted scroll stream.
Usage: python3 tools/measure_cpu.py <b-sw-below|b-gpu-below> <main|virtual2x> <tag>   (virtual2x needs tools/out/vdisplay running)
Runs the app with SPIKE720_SELFTEST (scroll stream at ~10-13 s) and SPIKE720_EXIT_AFTER, samples ps cumulative CPU time every
0.5 s, prints CPU%% as percent of one core. GPU load and power are NOT measured. Hard-coded repo path below.
"""
import subprocess, sys, time, os, re
mode, screen, tag = sys.argv[1], sys.argv[2], sys.argv[3]
root='/Users/jasonkulaga/dev/spike-719-windows-cef'
env=dict(os.environ, SPIKE720_SELFTEST='1', SPIKE720_EXIT_AFTER='17')
prof=f'{root}/results/cpu-{tag}'; subprocess.run(['rm','-rf',prof])
p=subprocess.Popen([f'{root}/target/bundle/spike720.app/Contents/MacOS/spike720',f'--mode={mode}',f'--screen={screen}',f'--profile={prof}'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
def cpu(s):  # [[d:]h:]m:ss.cc
    parts=s.split(':'); t=0.0
    for x in parts: t=t*60+float(x)
    return t
def snap():
    out=subprocess.run(['ps','-axo','pid=,ppid=,time=,rss=,command='],capture_output=True,text=True).stdout
    rows={}
    for l in out.splitlines():
        m=re.match(r'\s*(\d+)\s+(\d+)\s+(\S+)\s+(\d+)\s+(.*)',l)
        if m: rows[int(m.group(1))]=(int(m.group(2)),cpu(m.group(3)),int(m.group(4)),m.group(5))
    tree={p.pid}; ch=True
    while ch:
        ch=False
        for pid,(pp,_,_,_) in rows.items():
            if pp in tree and pid not in tree: tree.add(pid); ch=True
    role={}
    for pid in tree:
        if pid not in rows: continue
        c=rows[pid][3]
        r='gpu' if '--type=gpu-process' in c else 'renderer' if '--type=renderer' in c else 'utility' if '--type=utility' in c else 'browser' if pid==p.pid else 'other'
        role[pid]=r
    return time.time(),{pid:(rows[pid][1],rows[pid][2],role[pid]) for pid in role}
S=[]; t0=time.time()
while p.poll() is None and time.time()-t0<22:
    S.append(snap()); time.sleep(0.5)
S=[(t-t0,d) for t,d in S]
def window(a,b):
    pts=[(t,d) for t,d in S if a<=t<=b]
    if len(pts)<2: return None
    (ta,da),(tb,db)=pts[0],pts[-1]; agg={}; rss=0
    for pid,(c1,r1,role) in db.items():
        c0=da.get(pid,(0,0,role))[0]; agg[role]=agg.get(role,0)+(c1-c0)
    rss=sum(v[1] for v in db.values())/1024
    dt=tb-ta; return {k:100*v/dt for k,v in agg.items()}, rss
for name,(a,b) in {'idle (spinner animating)':(2.5,5.5),'scroll stream':(10.6,13.2)}.items():
    r=window(a,b)
    if not r: print(tag,name,'no data'); continue
    agg,rss=r; tot=sum(agg.values())
    print('%-9s %-24s total CPU %5.1f%% | %s | RSS %.0f MB'%(tag,name,tot,' '.join('%s %.1f'%(k,v) for k,v in sorted(agg.items())),rss))
p.wait(timeout=30)
