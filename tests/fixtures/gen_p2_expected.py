"""Channel-rack and player known answers: steps on the integer tick-to-sample rule, edits and
undo, plan swaps, tempo changes. Prints every case, (a) to (e). Run: python3 -I gen_p2_expected.py"""
import json, hashlib, math, copy
PPQ=96
STEP_VELOCITY=100/128   # velocity of a step switched on: 100 on the 0..128 scale
NEW_CHANNEL_VOLUME=0.5625   # the volume a channel that plays notes is added at: 9/16, -5.0 dB
def t2s(tick, sr, mbpm): return tick*60*sr*1000//(mbpm*PPQ)   # the one integer rule
def J(d): return json.dumps(d, separators=(',',':'))
def ch(i,n): return {"id":i,"name":n,"kind":"sampler","sample":"tests/fixtures/impulse.wav","volume":1.0,"pan":0.0,"mute":False}
def base(notes):
    return {"format_version":2,"sample_rate":44100,"tempo_mbpm":120000,"ppq":PPQ,"next_channel_id":2,
            "channels":[ch(1,"kick")],"patterns":[{"id":1,"name":"Pattern 1","length_bars":1,"notes":notes}]}
def note(c,step,vel=1.0): return {"channel":c,"tick":step*24,"length":0,"velocity":vel}
def order(n): return (n["tick"],n["channel"],n.get("key",60))
def toggle(d,p,c,step):
    # Returns the notes switched off; undoing that puts exactly those notes back.
    notes=d["patterns"][p-1]["notes"]; tk=step*24
    off=[n for n in notes if n["channel"]==c and n["tick"]==tk]
    if off: notes[:]=[n for n in notes if n not in off]; return off
    notes.append(note(c,step,STEP_VELOCITY)); notes.sort(key=order); return []
def put_back(d,p,removed):
    notes=d["patterns"][p-1]["notes"]; notes.extend(removed); notes.sort(key=order)
# (a)
e=base([]); toggle(e,1,1,5)
print("(a) empty + ToggleStep(p1,ch1,step5):", J(e["patterns"][0]["notes"]), " tick=5*24=",5*24)
f1=base([note(1,s) for s in (0,4,8,12)]); before=J(f1); removed=toggle(f1,1,1,4)
print("(a) F1 step4 off -> notes:", J(f1["patterns"][0]["notes"])); put_back(f1,1,removed)
print("(a) inverse restores identical:", J(f1)==before)
# (b)
d=copy.deepcopy(base([note(1,s) for s in (0,4,8,12)])); states=[J(d)]
def e1(d): toggle(d,1,1,2)
def e2(d): toggle(d,1,1,4)
def e3(d): d["channels"][0]["volume"]=0.5
def e4(d): d["tempo_mbpm"]=140000
def e5(d): d["channels"].append(dict(ch(2,"snare"),volume=NEW_CHANNEL_VOLUME)); d["next_channel_id"]=3
def e6(d): toggle(d,1,2,4)
def e7(d): d["patterns"][0]["length_bars"]=2
for f in (e1,e2,e3,e4,e5,e6,e7): f(d); states.append(J(d))
print("(b) final doc:", states[-1])
for k,s in enumerate(states): print("(b) state",k,"sha256[:16]",hashlib.sha256(s.encode()).hexdigest()[:16])
# (c)
p1=[0,4,8,12]; p2=[2,6]
print("(c) p1 ticks before",[s*24 for s in p1],"after toggle step2",sorted(s*24 for s in p1+[2]),
      "samples",[t2s(s*24,44100,120000) for s in sorted(p1+[2])])
print("(c) p2 ticks",[s*24 for s in p2],"samples",[t2s(s*24,44100,120000) for s in p2],"p3 empty")
# (d)
sr=44100;m=120000;blk=2450;swap=22050
print("(d) 2 bars =",t2s(768,sr,m),"frames =",t2s(768,sr,m)//blk,"blocks of",blk,"; swap before block starting at",swap,"=block",swap//blk)
A=[0,4,8,12]; B=sorted(A+[2,6]); bar=t2s(384,sr,m)
bar1=[t2s(s*24,sr,m) for s in A if t2s(s*24,sr,m)<swap]+[t2s(s*24,sr,m) for s in B if t2s(s*24,sr,m)>=swap]
bar2=[bar+t2s(s*24,sr,m) for s in B]
print("(d) bar length",bar,"expected impulses",bar1+bar2,"count",len(bar1+bar2))
vel=lambda s: 1.0 if s in A else STEP_VELOCITY   # A's own hits keep 1.0; steps 2 and 6 were switched on
print("(d) impulse values",[(t2s(s*24,sr,m),vel(s)) for s in A if t2s(s*24,sr,m)<swap]+[(t2s(s*24,sr,m),vel(s)) for s in B if t2s(s*24,sr,m)>=swap]+[(bar+t2s(s*24,sr,m),vel(s)) for s in B])
print("(d) step2 tick48 sample",t2s(48,sr,m),"< swap, must NOT fire late in bar 1; step6 tick144 sample",t2s(144,sr,m),"fires once")
# (e)
Bs=30000; old=120000; new=140000
t0=-(-Bs*old*PPQ//(60*sr*1000)); S0=t2s(t0,sr,old)
def tm(tick): return t2s(tick,sr,old) if tick<t0 else S0+(tick-t0)*60*sr*1000//(new*PPQ)
ev=[b*384+s*24 for b in (0,1) for s in A]
print("(e) buffer 1000, SetTempo 140000 queued before sample",Bs,"; t0=",t0,"S0=",S0)
print("(e) impulse ticks",ev); print("(e) impulse samples",[tm(t) for t in ev]); print("(e) 2 bars frames",tm(768))
print("(e) without change would be",[t2s(t,sr,old) for t in ev])
