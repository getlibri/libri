#!/usr/bin/env python3
"""Exercise all chat detail modes against the shared chat renderer."""
from pathlib import Path
import sys
import re
from throwaway_libri import Throwaway

FIXTURE = '''import json,sys,time
def emit(v): print(json.dumps(v),flush=True)
emit({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"Review the configuration before changing it."}}})
emit({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"I will check the configuration first."}}})
for i,name,args,result in [
 ("edit","Edit",{"file_path":"src/settings.rs","old_string":"const ENABLED: bool = false;","new_string":"const ENABLED: bool = true;"},"Updated src/settings.rs"),
 ("test","Bash",{"command":"cargo test settings","description":"Check the configuration"},"3 tests passed"),
 ("todo","TodoWrite",{"todos":[{"content":"Update configuration","activeForm":"Updating configuration","status":"completed"},{"content":"Check the result","activeForm":"Checking the result","status":"completed"}]},"Plan complete")]:
 emit({"type":"assistant","message":{"content":[{"type":"tool_use","id":i,"name":name,"input":args}]}})
 emit({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":i,"content":result}]}})
time.sleep(2)
emit({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"The configuration is updated. All three checks passed."}}})
emit({"type":"result","result":"The configuration is updated. All three checks passed.","is_error":False})
'''

def main():
 from playwright.sync_api import sync_playwright
 out=Path(sys.argv[3]);out.mkdir(parents=True,exist_ok=True)
 rig=Throwaway(sys.argv[1],prefix='libri-detail-',site=sys.argv[2])
 try:
  (rig.root/'claude').write_text(f'#!{sys.executable}\n'+FIXTURE)
  with sync_playwright() as p:
   browser=p.chromium.launch(headless=True)
   for size,width in [('desktop',1440),('phone',390)]:
    page=browser.new_page(viewport={'width':width,'height':900})
    page.add_init_script('localStorage.setItem("chat-detail","simple")')
    errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
    page.goto(rig.base)
    box=page.locator('textarea[name=input]:visible').first
    box.fill('Check the configuration '+rig.mark);box.press('Enter')
    page.get_by_role('img',name='Working',exact=True).first.wait_for()
    page.wait_for_timeout(500)
    assert page.locator('details[class*="group/tool"]:visible').count()==0
    assert not page.get_by_text('I will check the configuration first.',exact=True).first.is_visible()
    page.get_by_text('The configuration is updated. All three checks passed.',exact=True).last.wait_for(timeout=30000)
    for mode in ['full','simple','standard']:
     # Settings: the dock's gear on a wide screen; on a phone, a row of the side panel.
     if width<800:page.get_by_role('button',name='Side panel',exact=True).first.click();page.locator("[role=region][aria-label='Side panel'] [data-open=settings]").click()
     else:page.get_by_role('button',name='Settings and usage',exact=True).click()
     page.get_by_role('navigation',name='Settings categories').get_by_role('button',name=re.compile('^Chat')).click()
     page.locator('#chat-detail').select_option(mode)
     for _ in range(1 if width>=800 else 3):
      page.keyboard.press('Escape');page.wait_for_timeout(220)
     page.wait_for_timeout(300)
     cards=page.locator('details[class*="group/tool"]:visible')
     if mode=='full':
      assert cards.count()>=3
      assert cards.evaluate_all('(xs) => xs.every(x => x.open)')
      assert page.get_by_text('3 tests passed',exact=True).first.is_visible()
     elif mode=='simple':
      assert page.locator('[data-chat-detail="simple"]:visible').count()==0
      assert cards.count()==0
      assert not page.get_by_text("Update configuration",exact=True).first.is_visible()
      assert not page.get_by_text('3 tests passed',exact=True).first.is_visible()
      assert not page.get_by_text('Review the configuration before changing it.',exact=True).first.is_visible()
      assert not page.get_by_text('I will check the configuration first.',exact=True).first.is_visible()
      page.reload()
      page.get_by_text('The configuration is updated. All three checks passed.',exact=True).last.wait_for()
      assert not page.get_by_text('I will check the configuration first.',exact=True).first.is_visible()
     else:assert cards.count()==0 or cards.evaluate_all('(xs) => xs.every(x => !x.open)')
     assert page.evaluate('localStorage.getItem("chat-detail")')==mode
     page.screenshot(mask=[page.locator('[data-said=user]')],mask_color='#ede7dd',path=str(out/f'detail-{mode}-{size}.png'))
     assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), 'page overflows viewport'
    assert not errors,errors
    page.close()
   browser.close()
  assert not rig.strays()
  print('PASS: Full expanded input/output, Simple answers only live and reloaded, Standard folds; desktop and phone')
 finally:rig.close()

if __name__=='__main__':main()
