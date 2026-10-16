"""Original 128 BPM / 16-bar electronic pulse. Circular synthesis, no samples."""
import math, wave, array
SR=48000; N=SR*30; beat=.46875
out=[array.array('f',[0])*N for _ in range(2)]
def tone(start,dur,freq,amp,pan=.5,kind='bell'):
 for j in range(int(dur*SR)):
  u=j/SR
  env=(1-math.exp(-u*60))*math.exp(-u*(3.2 if kind=='bell' else 1.9))
  v=(math.sin(2*math.pi*freq*u)+.22*math.sin(2*math.pi*freq*2*u))*env*amp
  idx=(int(start*SR)+j)%N
  out[0][idx]+=v*math.sqrt(1-pan);out[1][idx]+=v*math.sqrt(pan)
# A warm Cmaj9 / Am9 / Fmaj9 / Gsus progression; all tails wrap.
chords=[[130.8128,164.8138,195.9977,246.9417],[110,130.8128,164.8138,195.9977],[87.3071,130.8128,164.8138,195.9977],[97.9989,146.8324,195.9977,220]]
for b in range(64):
 chord=chords[(b//16)%4]
 tone(b*beat,3,chord[b%4]*2,.12,.25+.5*(b%3)/2)
 if b%2==0:tone(b*beat,1,chord[0]/2,.18,.5,'bass')
 if b%4==2:tone(b*beat,2,chord[2]*4,.045,.7)
# Groove with alternating syncopation, swing, ghost snares and turnarounds.
for step in range(128):
 bar=step//8;pos=step%8
 start=step*beat/2+(beat*.055 if step%2 else 0)
 kick_on=pos in ([0,3,6] if bar%4==1 else [0,4,7] if bar%4==2 else [0,4])
 snare_on=pos in [2,6];ghost=pos==5 and bar%2==1
 fill=bar%4==3 and pos>=5
 velocity=.8+.2*math.sin(step*1.73)**2
 for j in range(int(.26*SR)):
  u=j/SR;idx=(int(start*SR)+j)%N
  kick=math.sin(2*math.pi*(48*u+8*(1-math.exp(-u*35))))*math.exp(-u*19)*.27 if kick_on else 0
  hat=math.sin(2*math.pi*7301*u)*math.sin(2*math.pi*5123*u)*math.exp(-u*(45 if pos==7 else 100))*.045*velocity
  snare=math.sin(2*math.pi*1777*u)*math.sin(2*math.pi*2341*u)*math.exp(-u*32)*(.10 if snare_on else .025 if ghost or fill else 0)
  tom=math.sin(2*math.pi*(100+pos*13)*u)*math.exp(-u*25)*.07 if fill else 0
  for c in range(2):out[c][idx]+=kick*velocity+hat+snare+tom
# slow pad uses integer cycles over the whole period, guaranteeing exact seam.
for i in range(N):
 t=i/SR
 for c in range(2):
  out[c][i]+=.018*sum(math.sin(2*math.pi*round(f*30)*t/30+c*.13) for f in [130.8128,195.9977,246.9417])*(.8+.2*math.cos(2*math.pi*t/30))
peak=max(max(abs(v) for v in ch) for ch in out); pcm=array.array('h')
for i in range(N):
 for c in range(2):pcm.append(round(out[c][i]/peak*.58*32767))
with wave.open('public/flow-v3.wav','wb') as w:w.setnchannels(2);w.setsampwidth(2);w.setframerate(SR);w.writeframes(pcm.tobytes())
print('30 seconds, stereo PCM, peak',peak)
