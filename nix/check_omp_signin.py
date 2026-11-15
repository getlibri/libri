#!/usr/bin/env python3
"""Exercise provider login through a throwaway Libri and stand-in omp.

No real provider is contacted and credentials never leave the stand-in.
Usage: check_omp_signin.py LIBRI_DIR [SCREENSHOT_DIR]
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.error
import urllib.request
from throwaway_libri import Throwaway, until

MOCK = '''import json,sys,time
for line in sys.stdin:
 c=json.loads(line)
 if c.get("type")=="login":
  print(json.dumps({"type":"extension_ui_request","method":"open_url","url":"https://login.example.com/device","instructions":"Enter code: TEST-CODE"}),flush=True)
  time.sleep(1)
  print(json.dumps({"type":"response","id":"login","success":True}),flush=True)
'''

def main():
 rig = Throwaway(sys.argv[1], prefix="libri-omp-signin-")
 try:
  rig.daemon.terminate(); rig.daemon.wait()
  mock=rig.root/'omp'
  mock.write_text(f'#!{sys.executable}\n'+MOCK);mock.chmod(0o755)
  env=rig.env | {"LIBRI_OMP":str(mock),"LIBRI_OMP_PROFILE":"proof","LIBRI_HARNESSES":"claude,omp"}
  rig.daemon=subprocess.Popen([rig.libri],env=env,stdout=subprocess.DEVNULL,stderr=open(rig.root/'signin.log','w'))
  until(rig.up,'scratch Libri up')
  def api(path,body=None):
   req=urllib.request.Request(rig.base+'/'+path,data=json.dumps(body).encode() if body else None,headers={'Content-Type':'application/json'})
   with urllib.request.urlopen(req) as r:return json.load(r)
  for provider in ['openai-codex-device','kimi-code','kilo']:
   r=api('api/signin',{'harness':'omp','provider':provider})
   assert r=={'url':'https://login.example.com/device','code':'TEST-CODE'},r
   until(lambda:api('api/signin/progress')['omp']['state']=='done','login completion')
  for provider in ['anthropic','google-gemini-cli','google-antigravity','unknown']:
   try:api('api/signin',{'harness':'omp','provider':provider})
   except urllib.error.HTTPError as e:assert e.code==502,e.code
   else:raise AssertionError('unsafe provider accepted')
  if len(sys.argv)>2:
   from playwright.sync_api import sync_playwright
   out=Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
   with sync_playwright() as p:
    browser=p.chromium.launch(headless=True)
    for name,width in [('desktop',1440),('phone',390)]:
     page=browser.new_page(viewport={'width':width,'height':900})
     page.goto(rig.base);page.wait_for_timeout(1000)
     page.get_by_role('button',name='Settings and usage',exact=True).click()
     if width < 800: page.get_by_role('button',name='Settings',exact=True).click()
     page.get_by_role('button',name='Models',exact=False).first.click()
     page.get_by_role('button',name='Sign in',exact=True).click()
     page.get_by_text('Waiting for omp to finish signing in…').wait_for()
     page.get_by_role('region',name='Models',exact=True).evaluate('(el) => { el.scrollTop = 0; }')
     page.screenshot(path=str(out/f'signin-{name}.png'))
     page.close()
    browser.close()
  assert not rig.strays(),'scratch run wrote outside its root'
  print('PASS: three provider flows, completion, four blocked providers, credential isolation')
 finally:rig.close()

if __name__=='__main__':main()
