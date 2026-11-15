#!/usr/bin/env python3
"""Android parity proof on an already booted emulator.
Run inside nix develop .#android-emulator:
  python nix/check_android_parity.py SERVER_PACKAGE DEBUG_APK [SERIAL]
All servers, memory, harnesses and credentials are throwaway. No real model runs.
"""
import os,sys,base64,zlib,struct
import json,subprocess,time,urllib.request,re,xml.etree.ElementTree as ET
from throwaway_libri import Throwaway,until
from check_omp_rendering import MOCK
from drive_ui import seed_chat
from pathlib import Path
SDK=os.environ['ANDROID_HOME']
ADB=SDK+'/platform-tools/adb';APP='is.libri';SERIAL=sys.argv[3] if len(sys.argv)>3 else 'emulator-5618'
ROOT=Path(__file__).resolve().parent.parent
OUT=ROOT/'docs/android-parity-captures';OUT.mkdir(exist_ok=True)
rig={};base=''

def fixture():
 r=Throwaway(sys.argv[1],prefix='libri-android-parity-')
 r.daemon.terminate();r.daemon.wait()
 r.hippo.terminate();r.hippo.wait()
 r.hippo=subprocess.Popen([r.hippo_bin,'serve','code'],env=r.env|{'HIPPO_BACKEND':'none'},stdout=subprocess.DEVNULL,stderr=open(r.root/'hippo.log','w'))
 until(lambda:(r.root/'hippo/hippo.sock').exists(),'hippo up')
 def chunk(k,v):return struct.pack('!I',len(v))+k+v+struct.pack('!I',zlib.crc32(k+v))
 png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!2I5B',240,100,8,2,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+b'\x26\x73\x4d'*240)*100))+chunk(b'IEND',b'')
 mock=MOCK.replace("  delta('thinking_delta'", "  delta('text_delta','I will check the configuration first.')\n  time.sleep(3)\n  delta('thinking_delta'")
 mock=mock.replace(" if c.get('type')=='get_state':", " if c.get('type')=='login':\n  emit({'type':'extension_ui_request','method':'open_url','url':'https://login.example.com/device','instructions':'Enter code: TEST-CODE'})\n  time.sleep(4)\n  emit({'type':'response','id':'login','success':True})\n elif c.get('type')=='get_state':")
 p=r.root/'omp';p.write_text(f'#!{sys.executable}\nROOT=__import__("pathlib").Path({str(r.root)!r})\nIMAGE={base64.b64encode(png).decode()!r}\n'+mock);p.chmod(0o755)
 env=r.env|{'LIBRI_BACKEND':'omp','LIBRI_HARNESSES':'omp','LIBRI_OMP':str(p),'LIBRI_OMP_PROFILE':'proof'}
 r.daemon=subprocess.Popen([r.libri],env=env,stdout=subprocess.DEVNULL,stderr=open(r.root/'omp.log','w'))
 until(r.up,'up')
 seed_chat(r,40,1)
 return r

def adb(*args):return subprocess.check_output([ADB,'-s',SERIAL,*args],text=True)
def api(path,body=None):
 req=urllib.request.Request(base+'/'+path,data=None if body is None else json.dumps(body).encode(),headers={'Content-Type':'application/json'})
 with urllib.request.urlopen(req,timeout=30) as r:return json.load(r) if r.headers.get('Content-Type','').startswith('application/json') else None
def screen():
 adb('shell','uiautomator','dump','/data/local/tmp/parity.xml')
 return adb('shell','cat','/data/local/tmp/parity.xml')
def shot(name):
 (OUT/(name+'.png')).write_bytes(subprocess.check_output([ADB,'-s',SERIAL,'exec-out','screencap','-p']))
def wait(test,timeout=30):
 end=time.monotonic()+timeout
 while time.monotonic()<end:
  x=screen()
  if test(x):return x
  time.sleep(.3)
 raise AssertionError('screen condition not met: '+x[:250])
def tap(label):
 x=screen();nodes=ET.fromstring(x).iter('node')
 for n in nodes:
  if n.get('text')==label or n.get('content-desc')==label:
   a,b,c,d=map(int,re.findall(r'\d+',n.get('bounds')));adb('shell','input','tap',str((a+c)//2),str((b+d)//2));return
 raise AssertionError('missing '+label)
def open_temp():
 # Temp is a bar tab once added; the phone bar starts without it, so the plus's menu adds it.
 x=screen()
 if any(n.get('text')=='Temp' or n.get('content-desc')=='Temp' for n in ET.fromstring(x).iter('node')):tap('Temp');return
 for n in ET.fromstring(x).iter('node'):
  if (n.get('content-desc') or '').startswith('Add to'):
   a,b,c,d=map(int,re.findall(r'\d+',n.get('bounds')));adb('shell','input','tap',str((a+c)//2),str((b+d)//2));break
 else:raise AssertionError('missing the plus')
 time.sleep(1);tap('Temp chat')
def open_worker(name):
 xml=wait(lambda x:name in x or 'SLEEPING' in x)
 if name not in xml:
  label=next(n.get('text') for n in ET.fromstring(xml).iter('node') if n.get('text','').startswith('SLEEPING'))
  tap(label)
 wait(lambda x:name in x);tap(name)

def no_gap(xml):
 nodes=list(ET.fromstring(xml).iter('node'))
 final=[n for n in nodes if 'The configuration is updated.' in n.get('text','')]
 boxes=[n for n in nodes if n.get('class')=='android.widget.EditText']
 assert final and boxes
 bottom=max(int(re.findall(r'\d+',n.get('bounds'))[3]) for n in final)
 top=min(int(re.findall(r'\d+',n.get('bounds'))[1]) for n in boxes)
 assert 0 <= top-bottom < 200,(top,bottom)

def launch(mode):
 adb('shell','am','force-stop',APP)
 prefs=f'<?xml version="1.0" encoding="utf-8"?><map><string name="address">{base}</string><string name="chat-detail">{mode}</string></map>'
 p=Path(rig['root'])/'prefs.xml';p.write_text(prefs)
 adb('push',str(p),'/data/local/tmp/parity-prefs.xml')
 adb('shell','run-as',APP,'sh','-c',"'mkdir -p shared_prefs && cp /data/local/tmp/parity-prefs.xml shared_prefs/libri.xml'")
 adb('shell','am','start','-W','-n',APP+'/su.libri.MainActivity');time.sleep(2)
def send():
 api('api/run',{'threadId':'main','runId':str(time.time()),'state':{},'tools':[],'context':[],'forwardedProps':{},'messages':[{'id':str(time.time()),'role':'user','content':'Verify configuration'}]})
if __name__=='__main__':
 server=fixture();rig={'root':str(server.root)};base=server.base
 try:
  adb('install','-r',sys.argv[2])
  port=base.rsplit(':',1)[1];adb('reverse','tcp:'+port,'tcp:'+port)
  for mode in ['simple','standard','full']:
   launch(mode);tap('Chat');time.sleep(1);send()
   if mode == 'simple':
    live=screen()
    assert 'I will check the configuration first.' not in live
    assert 'Updated src/settings.rs' not in live
    shot('simple-live')
   end=time.monotonic()+45
   while api('api/task')['typing'] and time.monotonic()<end:time.sleep(.3)
   time.sleep(1)
   xml=wait(lambda x:'The configuration is updated.' in x,45)
   if mode=='simple':
    assert 'I will check the configuration first.' not in xml
    assert '3 tests passed' not in xml
   shot(mode)
   if mode=='full':
    xml=wait(lambda x:'Tool image' in x,10)
    shot('full')
    assert '42.5 tok/s' in xml
    assert '32,000 / 128,000' in xml
    seen=xml
    for i in range(3):
     adb('shell','input','swipe','530','600','530','1600','450');time.sleep(.5)
     shot('full-older-'+str(i));seen+=screen()
    assert '3 tests passed' in seen
    assert 'ENABLED' in seen
    assert 'Configuration review passed.' in seen
   if mode=='simple':
    launch(mode);tap('Chat');xml=wait(lambda x:'The configuration is updated.' in x)
    assert 'I will check the configuration first.' not in xml
    no_gap(xml);shot('simple-reloaded')
    tap('Workers');tap('Chat');xml=wait(lambda x:'The configuration is updated.' in x);no_gap(xml);shot('simple-returned')
   print('PASS',mode,flush=True)
  tap('Side panel');tap('Settings');tap('Models')
  wait(lambda x:'Sign in with ChatGPT' in x);shot('signin-options')
  for title in ['ChatGPT','Kimi Code','Kilo']:
   tap('Sign in with '+title);wait(lambda x:'TEST-CODE' in x);shot('signin-'+title.lower().replace(' ','-'))
   wait(lambda x:'Signed in. Credentials stay in omp.' in x)
  shot('signin-done')
  launch('full');tap('Chat');open_temp()
  api('api/scratchpad',{'text':'Check configuration','files':[]})
  wait(lambda x:'Tool image' in x,50);wait(lambda x:'42.5 tok/s' in x,50);shot('full-temp')
  launch('full');tap('Workers')
  full_worker=api('api/work',{'brief':'Inspect configuration','files':[],'pick':{'harness':'omp','model':'openai-codex/test-model'}})
  wait(lambda x:full_worker['name'] in x);tap(full_worker['name'])
  wait(lambda x:'Tool image' in x,50);wait(lambda x:'42.5 tok/s' in x,50);shot('full-worker')
  launch('simple');tap('Chat');open_temp()
  api('api/scratchpad',{'text':'Check configuration','files':[]})
  x=screen();assert 'I will check the configuration first.' not in x;shot('simple-temp-live')
  until(lambda:not api('api/scratchpad')['working'],'Temp turn finished',wait=50)
  x=wait(lambda x:'The configuration is updated.' in x,50);assert '3 tests passed' not in x;shot('simple-temp')
  adb('shell','am','force-stop',APP);adb('shell','am','start','-W','-n',APP+'/su.libri.MainActivity');time.sleep(2)
  open_temp();x=wait(lambda x:'The configuration is updated.' in x)
  assert 'I will check the configuration first.' not in x;shot('simple-temp-reloaded')
  launch('simple');tap('Workers')
  worker=api('api/work',{'brief':'Check configuration','files':[],'pick':{'harness':'omp','model':'openai-codex/test-model'}})
  wait(lambda x:worker['name'] in x);tap(worker['name'])
  x=screen();assert 'I will check the configuration first.' not in x;shot('simple-worker-live')
  x=wait(lambda x:'The configuration is updated.' in x,50);assert '3 tests passed' not in x;shot('simple-worker')
  adb('shell','am','force-stop',APP);adb('shell','am','start','-W','-n',APP+'/su.libri.MainActivity');time.sleep(2)
  tap('Workers');open_worker(worker['name']);x=wait(lambda x:'The configuration is updated.' in x)
  assert 'I will check the configuration first.' not in x;shot('simple-worker-reloaded')
  assert not server.strays()
  print('PASS all Android parity proofs',flush=True)
 finally:
  adb('shell','am','force-stop',APP)
  server.close()
