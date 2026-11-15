#!/usr/bin/env python3
"""Prove omp RPC presentation through a scratch Libri and browser."""
import base64
import io
import json
from pathlib import Path
import subprocess
import sys
from throwaway_libri import Throwaway, until

MOCK = '''import json,sys,time
from pathlib import Path
if 'models' in sys.argv:
 print(json.dumps({"models":[{"kind":"chat","selector":"openai-codex/test-model","provider":"openai-codex","name":"Test model","thinking":["high"]}]}));sys.exit()
def emit(v):print(json.dumps(v),flush=True)
def delta(kind,text):emit({"type":"message_update","assistantMessageEvent":{"type":kind,"delta":text}})
def call(i,name,args,result):
 emit({"type":"tool_execution_start","toolCallId":i,"toolName":name,"args":args})
 emit({"type":"tool_execution_end","toolCallId":i,"toolName":name,"result":result,"isError":False})
for line in sys.stdin:
 c=json.loads(line)
 with open(ROOT/"requests.jsonl","a") as log: log.write(json.dumps(c)+"\\n")
 if c.get('type')=='get_state':
  emit({"type":"response","id":c['id'],"success":True,"data":{"contextUsage":{"tokens":32000,"contextWindow":128000},"tokensPerSecond":42.5}})
 elif c.get('type')=='prompt':
  (ROOT/'work/settings.rs').write_text('const ENABLED: bool = true;')
  delta('thinking_delta','Review the configuration before changing it.')
  call('edit','edit',{"path":"src/settings.rs","old_text":"const ENABLED: bool = false;","new_text":"const ENABLED: bool = true;"},{"content":[{"type":"text","text":"Updated src/settings.rs"}],"details":{"presentation_marker":"DISPLAY_ONLY_SENTINEL","path":"src/settings.rs","diff":"- const ENABLED: bool = false;\\n+ const ENABLED: bool = true;","oldText":"const ENABLED: bool = false;","newText":"const ENABLED: bool = true;"}})
  emit({"type":"tool_execution_start","toolCallId":"shell","toolName":"bash","args":{"command":"cargo test settings"}})
  emit({"type":"tool_execution_update","toolCallId":"shell","partialResult":{"content":[{"type":"text","text":"Checking configuration…"}]}})
  time.sleep(1.2)
  emit({"type":"tool_execution_end","toolCallId":"shell","toolName":"bash","result":{"content":[{"type":"text","text":"3 tests passed"}]},"isError":False})
  call('todo','todo',{"op":"done","task":"Check configuration"},{"content":[{"type":"text","text":"Overall: 1/1 done, 0 open."}],"details":{"phases":[{"name":"Verification","tasks":[{"content":"Check configuration","status":"completed"}]}]}})
  emit({"type":"tool_execution_start","toolCallId":"review","toolName":"task","args":{"tasks":[{"name":"Reviewer","agent":"reviewer","task":"Review configuration"}]}})
  emit({"type":"subagent_lifecycle","payload":{"id":"reviewer","parentToolCallId":"review","index":0,"status":"started"}})
  time.sleep(.7)
  emit({"type":"subagent_event","payload":{"id":"reviewer","event":{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Configuration review passed."}}}})
  emit({"type":"subagent_lifecycle","payload":{"id":"reviewer","parentToolCallId":"review","status":"completed"}})
  call('picture','read',{"path":"preview.png"},{"content":[{"type":"text","text":"Configuration preview"},{"type":"image","mimeType":"image/png","data":IMAGE}]})
  delta('text_delta','The configuration is updated. All three checks passed.')
  emit({"type":"message_end","message":{"role":"assistant","model":"test-model","usage":{"input":30000,"cacheRead":2000,"output":200}}})
  emit({"type":"prompt_result","id":c['id'],"status":"completed"})
'''

def main():
 from PIL import Image, ImageDraw
 from playwright.sync_api import sync_playwright
 out=Path(sys.argv[3]);out.mkdir(parents=True,exist_ok=True)
 rig=Throwaway(sys.argv[1],prefix='libri-omp-render-',site=sys.argv[2])
 try:
  rig.daemon.terminate();rig.daemon.wait()
  image=Image.new('RGB',(240,100),'#ede7dd');draw=ImageDraw.Draw(image)
  draw.rectangle((15,20,105,80),fill='#26734d');draw.rectangle((120,35,225,80),fill='#c99338')
  buf=io.BytesIO();image.save(buf,format='PNG')
  mock=rig.root/'omp';mock.write_text(f'#!{sys.executable}\nROOT=__import__("pathlib").Path({str(rig.root)!r})\nIMAGE={base64.b64encode(buf.getvalue()).decode()!r}\n'+MOCK);mock.chmod(0o755)
  env=rig.env | {'LIBRI_BACKEND':'omp','LIBRI_HARNESSES':'omp','LIBRI_OMP':str(mock)}
  rig.daemon=subprocess.Popen([rig.libri],env=env,stdout=subprocess.DEVNULL,stderr=open(rig.root/'omp.log','w'))
  until(rig.up,'scratch omp Libri up')
  with sync_playwright() as p:
   browser=p.chromium.launch(headless=True)
   for name,width in [('desktop',1440),('phone',390)]:
    page=browser.new_page(viewport={'width':width,'height':1000})
    page.add_init_script('localStorage.setItem("chat-detail","full")')
    page.goto(rig.base)
    box=page.locator('textarea[name=input]:visible').first;box.fill('Verify configuration '+rig.mark);box.press('Enter')
    page.get_by_text('Checking configuration…',exact=True).last.wait_for()
    page.locator('[data-agent-progress=running]').last.wait_for()
    page.get_by_text('The configuration is updated. All three checks passed.',exact=True).last.wait_for(timeout=30000)
    page.get_by_text('42.5 tok/s',exact=True).last.wait_for()
    assert page.get_by_role('progressbar',name='Model context').last.get_attribute('aria-valuenow')=='32000'
    assert page.get_by_role('progressbar',name='Model context').last.get_attribute('aria-valuemax')=='128000'
    assert page.get_by_text('3 tests passed',exact=True).last.is_visible()
    assert page.locator('[data-tool-diff]').last.is_visible()
    assert page.get_by_text('const ENABLED: bool = true;',exact=False).last.is_visible()
    page.get_by_alt_text('Tool image',exact=True).last.scroll_into_view_if_needed()
    page.get_by_alt_text('Tool image',exact=True).last.wait_for(state='visible')
    assert page.get_by_text('Check configuration',exact=True).last.is_visible()
    assert page.get_by_text('Configuration review passed.',exact=True).last.is_visible()
    image=page.get_by_alt_text('Tool image',exact=True).last;image.wait_for()
    assert image.evaluate('(img) => img.complete && img.naturalWidth > 0')
    page.screenshot(mask=[page.locator('[data-said=user]')],mask_color='#ede7dd',path=str(out/f'omp-full-{name}.png'))
    page.locator('[data-tool-diff]').last.scroll_into_view_if_needed()
    page.screenshot(mask=[page.locator('[data-said=user]')],mask_color='#ede7dd',path=str(out/f'omp-diff-{name}.png'))
    # History must supply final display records independently of the stream.
    history = page.request.get(rig.base+'/api/history').json()
    rich = [row for row in history if row.get('display', {}).get('version') == 1]
    assert any(row['display'].get('result', {}).get('details', {}).get('diff') for row in rich)
    assert any(row['display'].get('metrics', {}).get('tokens_per_second') == 42.5 for row in rich)
    image_records = [part for row in rich for part in row['display'].get('result', {}).get('content', []) if part.get('type') == 'image']
    assert image_records and all(part.get('file') and 'data' not in part for part in image_records)
    assert (rig.root/'work/settings.rs').read_text() == 'const ENABLED: bool = true;'
    rig.daemon.terminate(); rig.daemon.wait()
    rig.hippo.terminate(); rig.hippo.wait()
    rig.hippo=subprocess.Popen([rig.hippo_bin, 'serve', 'code'],env=rig.env,stdout=subprocess.DEVNULL,stderr=open(rig.root/'hippo-restart.log','w'))
    until(lambda: (rig.root/'hippo/hippo.sock').exists(), 'hippo restarted')
    rig.daemon=subprocess.Popen([rig.libri],env=env,stdout=subprocess.DEVNULL,stderr=open(rig.root/'omp-restart.log','w'))
    until(rig.up, 'Libri restarted')
    page.close()
    page=browser.new_page(viewport={'width':width,'height':1000})
    page.add_init_script('localStorage.setItem("chat-detail","full")')
    page.goto(rig.base)
    page.locator('[data-tool-diff]').last.wait_for(state='visible')
    page.get_by_alt_text('Tool image',exact=True).last.scroll_into_view_if_needed()
    until(lambda: page.get_by_alt_text('Tool image',exact=True).last.evaluate('(img) => img.complete && img.naturalWidth > 0'), 'restored image')
    page.get_by_text('42.5 tok/s',exact=True).last.wait_for()
    page.screenshot(mask=[page.locator('[data-said=user]')],mask_color='#ede7dd',path=str(out/f'omp-reloaded-{name}.png'))
    assert 'DISPLAY_ONLY_SENTINEL' not in (rig.root/'requests.jsonl').read_text()
    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
    page.close()
   browser.close()
  assert not rig.strays()
  print('PASS: omp thinking, tools, partial output path, todos, subagent report, inline image, actual context and speed, restart/reload, file references and model-context isolation; desktop and phone')
 finally:rig.close()

if __name__=='__main__':main()
