#!/usr/bin/env python3
"""Prove answer-only Temp and worker views against isolated host responses."""
import json
from pathlib import Path
import sys
from throwaway_libri import Throwaway

LOG = '\n'.join([
 '0|note: subagent', '1|user: Check configuration.',
 '2|talk: The first check passed.',
 '3|user: (From the person, typed on your page) Check again.',
 '4|talk: I will inspect the configuration first.',
 '5|tool: Edit {"file_path":"settings.rs","old_string":"false","new_string":"true"}',
 '6|echo: Updated settings.rs',
 '7|tool: TodoWrite {"todos":[{"content":"Check settings","status":"completed"}]}',
 '8|echo: Plan complete',
 '9|tool: Task {"prompt":"Review settings"}',
 '10|echo: Review complete',
])
FINAL = 'The configuration is updated.'

def main():
 from playwright.sync_api import sync_playwright
 out=Path(sys.argv[3]);out.mkdir(parents=True,exist_ok=True)
 rig=Throwaway(sys.argv[1],prefix='libri-simple-threads-',site=sys.argv[2])
 try:
  with sync_playwright() as p:
   browser=p.chromium.launch(headless=True)
   for size,width in [('desktop',1440),('phone',390)]:
    for kind in ['temp','thread']:
     state={'live':True}
     page=browser.new_page(viewport={'width':width,'height':900})
     errors=[];page.on('pageerror',lambda e:errors.append(str(e)))
     pane={'kind':kind,**({'arg':'fixture'} if kind=='thread' else {})}
     layout={'spaces':[{'id':'space','name':'Chat','root':{'t':'leaf','id':'leaf','pane':pane},'focus':'leaf','full':False}],'on':'space'}
     def reply(route):
      path=route.request.url.split(rig.base)[-1]
      log=LOG+('' if state['live'] else '\n11|talk: '+FINAL)
      if path.startswith('/api/phone-layout'):
       # A phone keeps its own bar (phone.tsx), the pane its one item.
       if '?' in path:return route.fulfill(status=204)
       data={'version':1,'layout':{'spaces':[{'id':'space','name':'Chat','items':[pane]}],'on':'space'},'undo':False}
      elif path.startswith('/api/layout'):
       if '?' in path:return route.fulfill(status=204)
       data={'version':1,'layout':layout,'undo':False}
      elif path=='/api/scratchpad':
       data={'id':'scratchpad-fixture','text':log,'working':state['live'],'words':'I will inspect the configuration first.' if state['live'] else FINAL,'thinking':'Inspect settings privately.','stopping':False,'queued':[],'memory':False,'asks':[]}
      elif path=='/api/work':
       data={'threads':[{'id':'fixture','name':'Reviewer','brief':'Check configuration.','state':'working' if state['live'] else 'waiting','summary':'Editing settings.rs','gist':'Check settings','at':0,'read':True}]}
      elif path=='/api/work/fixture':data={'text':log}
      else:return route.continue_()
      route.fulfill(json=data)
     page.route('**/api/**',reply)
     page.add_init_script('localStorage.setItem("chat-detail","simple")')
     page.goto(rig.base)
     surface=page.locator('[data-surface='+kind+']').last
     surface.wait_for()
     surface.get_by_role('img',name='Working',exact=True).wait_for()
     def hidden():
      assert surface.locator('details:visible').count()==0
      for text in ['I will inspect the configuration first.','Updated settings.rs','Plan complete','Review complete','Check settings','Editing settings.rs','Inspect settings privately.']:
       assert not surface.get_by_text(text,exact=True).first.is_visible(),text
     hidden()
     assert surface.get_by_text('The first check passed.',exact=True).first.is_visible()
     page.screenshot(path=str(out/f'simple-{kind}-live-{size}.png'))
     state['live']=False
     page.reload()
     surface.get_by_text(FINAL,exact=True).last.wait_for()
     hidden()
     assert surface.get_by_text('Check configuration.',exact=True).first.is_visible()
     page.screenshot(path=str(out/f'simple-{kind}-reloaded-{size}.png'))
     assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
     assert not errors,errors
     page.close()
   browser.close()
  assert not rig.strays()
  print('PASS: Simple Temp and worker threads hide all work live and after reload, desktop and 390px')
 finally:rig.close()

if __name__=='__main__':main()
