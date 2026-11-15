#!/usr/bin/env python3
"""Prove default model routing against an isolated catalog, without logins."""
import json
from pathlib import Path
import subprocess
import sys
import urllib.request
from throwaway_libri import Throwaway, until

PROVIDERS=['openai-codex','deepseek','xai','google','anthropic','google-gemini-cli','google-antigravity','meta']

def main():
 rig=Throwaway(sys.argv[1],prefix='libri-omp-catalog-')
 try:
  rig.daemon.terminate();rig.daemon.wait()
  models={'models':[{'kind':'chat','selector':p+'/test-model','provider':p,'name':p+' test','thinking':['low','high']} for p in PROVIDERS]}
  mock=rig.root/'omp';mock.write_text(f'#!{sys.executable}\nimport json\nprint({json.dumps(models)!r})\n');mock.chmod(0o755)
  env=rig.env | {'LIBRI_HARNESSES':'claude,codex,omp','LIBRI_OMP':str(mock),'LIBRI_CODEX':'/bin/false'}
  env['LIBRI_BACKEND']='omp'
  rig.daemon=subprocess.Popen([rig.libri],env=env,stdout=subprocess.DEVNULL,stderr=open(rig.root/'catalog.log','w'))
  until(rig.up,'scratch catalog up')
  with urllib.request.urlopen(rig.base+'/api/models') as r: data=json.load(r)
  assert data['pick']['harness']=='omp',data
  offered={o['provider'] for o in data['offers'] if o['harness']=='omp'}
  assert offered=={'openai-codex','deepseek','xai','google'},offered
  assert any(o['harness']=='claude' for o in data['offers'])
  if len(sys.argv)>2:
   from playwright.sync_api import sync_playwright
   out=Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
   with sync_playwright() as p:
    browser=p.chromium.launch(headless=True)
    for name,width in [('desktop',1440),('phone',390)]:
     page=browser.new_page(viewport={'width':width,'height':900})
     page.goto(rig.base);page.get_by_role('button',name='Settings and usage',exact=True).click()
     if width<800:page.get_by_role('button',name='Settings',exact=True).click()
     page.get_by_role('button',name='Models',exact=False).first.click()
     page.locator('#chat-model').wait_for()
     assert 'openai-codex test' in page.locator('#chat-model').inner_text()
     page.screenshot(path=str(out/f'catalog-{name}.png'));page.close()
    browser.close()
  assert not rig.strays()
  print('PASS: omp default, OpenAI on omp with Codex installed, restricted login exclusion, Claude retained')
 finally:rig.close()

if __name__=='__main__':main()
