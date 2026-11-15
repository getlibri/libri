#!/usr/bin/env python3
"""Exercise optional settings, rejected writes and canonical saves in a scratch UI."""
import json
import sys
from pathlib import Path
from throwaway_libri import Throwaway


def main():
    from playwright.sync_api import sync_playwright, expect
    output = Path(sys.argv[3])
    output.mkdir(parents=True, exist_ok=True)
    rig = Throwaway(sys.argv[1], prefix='libri-settings-', site=sys.argv[2])
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True)
            for width in [1440, 390]:
                page = browser.new_page(viewport={'width': width, 'height': 900})
                errors = []
                page.on('pageerror', lambda error: errors.append(str(error)))
                state = {'text': 'Saved rules', 'reject': True, 'missing': False}
                def guide(route):
                    if route.request.method == 'POST':
                        if state['reject']:
                            route.fulfill(status=400, body='Rules rejected')
                            return
                        state['text'] = json.loads(route.request.post_data)['text'].strip()
                    if state['missing']:
                        route.fulfill(status=404, body='Not found')
                    else:
                        route.fulfill(json={'text': state['text']})
                page.route('**/api/guide', guide)
                page.goto(rig.base)
                # Settings: the dock's gear on a wide screen; on a phone, a row of the side panel.
                def settings():
                    if width < 800:
                        page.get_by_role('button', name='Side panel', exact=True).first.click()
                        page.locator("[role=region][aria-label='Side panel'] [data-open=settings]").click()
                    else:
                        page.get_by_role('button', name='Settings and usage', exact=True).click()
                settings()
                page.get_by_role('navigation', name='Settings categories').get_by_role('button', name='Agents', exact=False).click()
                field = page.get_by_role('textbox', name='Your rules', exact=True)
                # The rules' own Save: the Agents page holds the host's facts and their Save too.
                save = field.locator('xpath=following-sibling::button[1]')
                expect(field).to_have_value('Saved rules')
                field.fill('  Revised rules  ')
                save.click()
                expect(page.get_by_role('alert').filter(has_text='Rules rejected')).to_be_visible()
                expect(field).to_have_value('  Revised rules  ')
                state['reject'] = False
                save.click()
                expect(field).to_have_value('Revised rules')
                expect(save).to_be_disabled()
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                page.screenshot(path=str(output / f'settings-{width}.png'))
                page.reload()
                settings()
                page.get_by_role('navigation', name='Settings categories').get_by_role('button', name='Agents', exact=False).click()
                expect(field).to_have_value('Revised rules')
                state['missing'] = True
                page.reload()
                settings()
                expect(page.get_by_role('navigation', name='Settings categories').get_by_role('button', name='Agents', exact=False)).to_have_count(0)
                assert not errors, errors
                page.close()
            browser.close()
        assert not rig.strays()
        print('PASS: optional setting, rejected save, canonical response and reload at 1440px and 390px')
    finally:
        rig.close()


if __name__ == '__main__':
    main()
