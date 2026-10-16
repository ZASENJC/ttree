import subprocess,json,array,wave,math
from pathlib import Path
D=Path(__file__).resolve().parent
p=D/'out/TTREE-30s.mp4'
def run(args):return subprocess.check_output(args)
info=json.loads(run(['ffprobe','-v','error','-show_streams','-show_format','-of','json',str(p)]))
v=next(x for x in info['streams'] if x['codec_type']=='video')
a=next(x for x in info['streams'] if x['codec_type']=='audio')
# Decode all frames at analysis resolution; compare seam against adjacent motion.
raw=run(['ffmpeg','-v','error','-i',str(p),'-vf','scale=192:108','-f','rawvideo','-pix_fmt','gray','-'])
size=192*108; frames=[raw[i:i+size] for i in range(0,len(raw),size)]
def mae(a,b):return sum(abs(x-y) for x,y in zip(a,b))/len(a)
# Decode delivery AAC as well as original PCM; assess boundary sample jump.
sound=array.array('f',run(['ffmpeg','-v','error','-i',str(p),'-vn','-f','f32le','-ar','48000','-ac','2','-']))
jump=max(abs(sound[c]-sound[-2+c]) for c in [0,1])
peak=max(abs(x) for x in sound)
black=[i for i,x in enumerate(frames) if sum(x)/len(x)<10]
result={'duration_seconds':float(info['format']['duration']),'width':v['width'],'height':v['height'],'fps':v['r_frame_rate'],'frames':len(frames),'audio_codec':a['codec_name'],'audio_hz':a['sample_rate'],'black_frames':black,'seam_mean_pixel_delta_255':mae(frames[-1],frames[0]),'adjacent_start_delta':mae(frames[0],frames[1]),'adjacent_end_delta':mae(frames[-2],frames[-1]),'aac_peak_linear':peak,'aac_boundary_jump_linear':jump,'aac_boundary_jump_dbfs':20*math.log10(max(jump,1e-12))}
assert len(frames)==900 and not black and abs(result['duration_seconds']-30)<.001
assert jump<.01 and peak<1
(D/'out/qa.json').write_text(json.dumps(result,indent=2))
print(json.dumps(result,indent=2))
