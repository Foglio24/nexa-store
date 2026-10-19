#!/usr/bin/env python3
"""UI check of Nexa Store on the real window.

It installs, removes and updates nothing: it searches, browses categories, opens details and
confirmations (then cancels them) and never runs an operation (NEXA_RUN_CHECK=1 also runs the update check). Needs the release binary, tauri-driver, WebKitWebDriver and a graphical session.
Usage: python3 scripts/test-gui.py   (NEXA_TAURI_DRIVER=/path/to/tauri-driver if it is not in PATH)

The interface follows the system language, so the script fixes it: English unless NEXA_LANG=it is set
(Italian), and the selectors below use the labels of that language.
"""
import base64, json, os, shutil, subprocess, sys, time, urllib.request, urllib.error
from pathlib import Path

PROJECT = Path(__file__).resolve().parent.parent
DRIVER = os.environ.get('NEXA_TAURI_DRIVER', shutil.which('tauri-driver') or 'tauri-driver')
BINARY = PROJECT / 'src-tauri/target/release/nexa-store'
OUT = PROJECT / 'verification'
OUT.mkdir(exist_ok=True)
PORT = 4466
BASE = f'http://127.0.0.1:{PORT}'
session = None
results = []


def req(method, route, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    r = urllib.request.Request(BASE + route, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(r, timeout=180) as resp:
            body = json.load(resp)
    except urllib.error.HTTPError as e:
        raise RuntimeError(e.read().decode()) from e
    return body.get('value', body)


def cmd(method, route, payload=None): return req(method, f'/session/{session}' + route, payload)
def js(script, args=None): return cmd('POST', '/execute/sync', {'script': script, 'args': args or []})


def wait(script, timeout=60):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = js(script)
        if value: return value
        time.sleep(0.3)
    raise AssertionError('timeout: ' + script[:140])


def shot(name): (OUT / name).write_bytes(base64.b64decode(cmd('GET', '/screenshot')))
def note(name, data):
    results.append({'check': name, 'result': data})
    print(name, '=>', json.dumps(data, ensure_ascii=False)[:420], flush=True)


def click_text(text, within='document'):
    found = js(f'const b=[...{within}.querySelectorAll("button")].find(b=>b.textContent.trim().startsWith(arguments[0]));if(b){{b.click();return true}}return false', [text])
    assert found, f'button not found: {text}'


def type_query(text):
    if not text:
        js('document.querySelector("button[aria-label=\\"' + L['clear'] + '\\"]")?.click()')
        return
    el = cmd('POST', '/element', {'using': 'css selector', 'value': '.search input'})['element-6066-11e4-a52e-4f735466cecf']
    cmd('POST', f'/element/{el}/clear', {})
    cmd('POST', f'/element/{el}/value', {'text': text})


def scroll_through():
    """Images below the fold are lazy-loaded: scroll so they count."""
    for _ in range(10):
        js('document.querySelector(".main").scrollBy(0,700)'); time.sleep(0.35)
    time.sleep(1.5)
    js('document.querySelector(".main").scrollTo(0,0)')


CARDS = '''return [...document.querySelectorAll(".app-card, .app-row")].map(c=>({
  name:c.querySelector(".app-name")?.childNodes[0]?.textContent, sources:[...c.querySelectorAll(".source-badge")].map(b=>b.textContent.trim()),
  tech:!!c.querySelector(".tag"), logo:(c.querySelector("img.app-logo")?.naturalWidth||0)>0, fallback:!!c.querySelector(".logo-fallback")}))'''


def settle(minimum=1):
    wait(f'return !document.querySelector("[aria-busy=true]") && !document.querySelector(".skeleton") && document.querySelectorAll(".app-card, .app-row").length>={minimum}', 90)
    time.sleep(2)


def modal_open(): return js('return !!document.querySelector("[role=dialog]")')


LANGUAGE = os.environ.get('NEXA_LANG', 'en')
L = {
    'en': dict(clear='Clear search', more='Show more apps', games='Games', source='Source', all='All', kind='Type', packages='Packages', everything='Everything',
               install='Install', cancel='Cancel', installed='Installed', updates='Updates', remove='Remove', open='Open ', utilities='Utilities', explore='Explore', check='Check for updates', operations='Operations', done='done'),
    'it': dict(clear='Cancella ricerca', more='Mostra altre app', games='Giochi', source='Sorgente', all='Tutte', kind='Tipo', packages='Pacchetti', everything='Tutto',
               install='Installa', cancel='Annulla', installed='Installate', updates='Aggiornamenti', remove='Rimuovi', open='Apri ', utilities='Utilità', explore='Esplora', check='Controlla aggiornamenti', operations='Operazioni', done='fatto'),
}[LANGUAGE]
env = dict(os.environ, GDK_BACKEND='x11', NEXA_LANG=LANGUAGE)
drv = subprocess.Popen([DRIVER, '--port', str(PORT), '--native-port', str(PORT + 1)], env=env,
                       stdout=(OUT / 'webdriver-gui.log').open('w'), stderr=subprocess.STDOUT)
ok = False
try:
    for _ in range(100):
        try: req('GET', '/status'); break
        except Exception: time.sleep(0.1)
    session = req('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': str(BINARY)}}}})['sessionId']
    cmd('POST', '/timeouts', {'script': 180000})

    # ---- Home: highlighted apps, categories, new proposals
    wait('return document.querySelector(".hero-body h2")?.textContent', 90)
    assert js('return document.documentElement.lang') == LANGUAGE, 'the interface is not in the expected language'
    wait('return document.querySelectorAll(".app-card").length>=20', 90)
    time.sleep(2)
    text = js('return document.body.innerText')
    assert 'fase 1' not in text.lower() and 'fatto per ubuntu' not in text.lower(), 'promotional text still present'
    tiles = js('return [...document.querySelectorAll(".category-tile")].map(t=>t.textContent.trim())')
    assert len(tiles) == 10, tiles
    note('home: categories', tiles)
    hero = js('return document.querySelector(".hero-body h2").textContent')
    first_picks = js('return [...document.querySelectorAll("#discover-title ~ .grid-cards .app-name, #discover-title + * .app-name")].map(x=>x.textContent)')
    sections = js('return [...document.querySelectorAll(".section-title")].map(x=>x.textContent.trim())')
    assert len(sections) >= 5, sections
    note('home: highlighted app and sections', {'highlighted': hero, 'sections': sections})
    wait('return document.querySelectorAll(".dot.on").length>=4')
    shot('10-home.png')
    cards = js(CARDS)
    scroll_through()
    cards = js(CARDS)
    with_logo = sum(c['logo'] for c in cards)
    note('home: logos', {'cards': len(cards), 'with_logo': with_logo})
    assert with_logo >= len(cards) * 0.85, 'too many cards without a logo'

    # A second launch (a second click on the menu icon) must be forwarded to the open window, not crash it.
    second = subprocess.run([str(BINARY)], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
    time.sleep(1.5)
    assert js('return !!document.querySelector(".hero-body h2")'), 'the window does not respond after a second launch'
    note('second launch forwarded to the open window', {'exit': second.returncode})

    # "Show more apps" proposes different apps.
    before = js('return [...document.querySelectorAll(".grid-cards")[0].querySelectorAll(".app-name")].map(x=>x.textContent)')
    click_text(L['more'])
    time.sleep(1)
    wait('return !document.querySelector("[aria-busy=true]")', 60); time.sleep(2)
    after = js('return [...document.querySelectorAll(".grid-cards")[0].querySelectorAll(".app-name")].map(x=>x.textContent)')
    assert before != after, 'the proposals do not change'
    note('show more apps: different proposals', {'before': before[:4], 'after': after[:4]})

    # ---- A category page
    js('[...document.querySelectorAll(".category-tile")].find(t=>t.textContent.trim()==="' + L['games'] + '").click()')
    wait('return document.querySelector("h1")?.textContent==="' + L['games'] + '"')
    settle(24)
    scroll_through()
    games = js(CARDS)
    count = js('return document.querySelector(".result-count")?.textContent')
    assert len(games) >= 24 and all(not c['tech'] for c in games)
    note('category Games', {'cards': len(games), 'total': count, 'with_logo': sum(c['logo'] for c in games), 'sources': sorted({s for c in games for s in c['sources']})})
    assert sum(c['logo'] for c in games) >= len(games) * 0.9
    shot('11-category-games.png')
    click_text('Snap', f'document.querySelector(".segmented[aria-label={L["source"]}]")')
    time.sleep(1); settle(12)
    only_snap = js(CARDS)
    assert only_snap and all(c['sources'] == ['Snap'] for c in only_snap), 'Snap filter not respected'
    note('category Games, Snap only', {'cards': len(only_snap), 'examples': [c['name'] for c in only_snap[:4]]})
    click_text(L['all'], f'document.querySelector(".segmented[aria-label={L["source"]}]")')
    js('document.querySelector(".back-link").click()')
    wait('return !!document.querySelector(".hero-body h2")')

    # ---- Search
    type_query('gimp'); settle(3)
    rows = js(CARDS)
    first = rows[0]
    assert first['name'] and ('GNU Image' in first['name'] or 'GIMP' in first['name'].upper()), f'the first result is not GIMP: {first}'
    assert {'APT', 'Flatpak', 'Snap'} <= set(first['sources']), first
    assert all(not r['tech'] for r in rows) and not any(r['name'] == 'gimp-data' for r in rows)
    assert first['logo'], 'GIMP without a logo'
    note('search gimp (apps only)', {'first': first, 'results': len(rows)})
    shot('12-search.png')
    click_text(L['packages'], f'document.querySelector(".segmented[aria-label={L["kind"]}]")'); settle(3)
    technical = js(CARDS)
    assert any(r['name'] == 'gimp-data' for r in technical) and all(r['tech'] for r in technical)
    assert all(r['fallback'] or r['logo'] for r in technical)
    note('search gimp (technical packages)', [r['name'] for r in technical[:5]])
    shot('13-search-packages.png')
    click_text(L['everything'], f'document.querySelector(".segmented[aria-label={L["kind"]}]")'); settle(3)
    click_text('App', f'document.querySelector(".segmented[aria-label={L["kind"]}]")'); settle(3)

    # ---- Details with a source choice
    js('document.querySelector(".app-row .app-row-main").click()')
    wait('return !!document.querySelector("[role=dialog] .detail-description")', 60)
    variants = js('return [...document.querySelectorAll("[role=dialog] .variant-picker button")].map(b=>b.textContent.trim())')
    assert len(variants) == 3, variants

    apt_description = js('return document.querySelector(".detail-description").textContent')
    assert len(apt_description) > 100 and '\n\n' in apt_description or len(apt_description) > 100
    click_text('Flatpak', 'document.querySelector("[role=dialog] .variant-picker")')
    time.sleep(2.5)
    wait('return document.querySelector(".detail-description")?.textContent.length>60', 60)
    flatpak_description = js('return document.querySelector(".detail-description").textContent')
    assert 'Ref:' not in flatpak_description and 'Commit:' not in flatpak_description, 'raw Flatpak description'
    assert not any('\u0400' <= ch <= '\u04ff' for ch in flatpak_description), 'Flatpak description in the wrong language'
    note('GIMP details', {'sources': variants, 'APT': apt_description[:70], 'Flatpak': flatpak_description[:70]})
    shot('14-details.png')

    # ---- Install confirmation: opened and cancelled
    click_text(L['install'], 'document.querySelector("[role=dialog] .detail-actions")')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 90)
    plan = js('return {title:document.querySelector(".confirm-title").textContent, source:document.querySelector(".confirm-head .source-badge")?.textContent.trim(), command:document.querySelector(".command-preview pre").textContent, button:document.querySelector("[data-testid=confirm-operation]").textContent}')
    assert plan['title'].startswith(L['install']) and plan['source'] == 'Flatpak' and 'flatpak' in plan['command']
    note('install confirmation (not executed)', {'title': plan['title'], 'source': plan['source'], 'button': plan['button']})
    shot('15-confirmation.png')
    click_text(L['cancel'])
    assert not modal_open()
    type_query('')

    # ---- AppImage: an app that exists only as an AppImage. The confirmation is opened and cancelled.
    type_query('zettlr'); settle(1)
    rows = js(CARDS)
    zettlr = next((r for r in rows if (r['name'] or '').lower() == 'zettlr'), None)
    assert zettlr and 'AppImage' in zettlr['sources'], rows[:3]
    note('search zettlr (has an AppImage)', zettlr)
    js('document.querySelector(".app-row .app-row-main").click()')
    wait('return !!document.querySelector("[role=dialog] .detail-description")', 60)
    if js('return !!document.querySelector("[role=dialog] .variant-picker")'):
        click_text('AppImage', 'document.querySelector("[role=dialog] .variant-picker")')
        time.sleep(2)
        wait('return !!document.querySelector("[role=dialog] .detail-description")', 60)
    assert js('return !!document.querySelector("[role=dialog] .snap-restriction")'), 'the AppImage warning is missing'
    click_text(L['install'], 'document.querySelector("[role=dialog] .detail-actions")')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 90)
    plan = js('return {source:document.querySelector(".confirm-head .source-badge")?.textContent.trim(), command:document.querySelector(".command-preview pre").textContent, details:[...document.querySelectorAll(".plan-details li")].map(li=>li.textContent)}')
    assert plan['source'] == 'AppImage' and 'download https://github.com/Zettlr/Zettlr/releases/download/' in plan['command'], plan
    assert 'pkexec' not in plan['command'] and any('SHA-256' in detail for detail in plan['details']), plan
    note('AppImage install confirmation (not executed)', {'command': plan['command'].splitlines()[0], 'details': len(plan['details'])})
    shot('15b-appimage-confirmation.png')
    click_text(L['cancel'])
    assert not modal_open()
    type_query('')
    js('[...document.querySelectorAll(".category-tile")].find(t=>t.textContent.trim()===arguments[0]).click()', [L['utilities']])
    wait('return document.querySelector("h1")?.textContent===arguments[0]'.replace('arguments[0]', json.dumps(L['utilities'])))
    settle(12)
    click_text('AppImage', f'document.querySelector(".segmented[aria-label={L["source"]}]")')
    time.sleep(1); settle(12)
    only_appimage = js(CARDS)
    assert only_appimage and all(c['sources'] == ['AppImage'] for c in only_appimage), only_appimage[:3]
    note('category Utilities, AppImage only', {'cards': len(only_appimage), 'examples': [c['name'] for c in only_appimage[:4]]})
    js('document.querySelector(".back-link").click()')
    wait('return !!document.querySelector(".hero-body h2")')

    # ---- Installed: real logos
    js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")==="' + L['installed'] + '").click()')
    wait('return document.querySelectorAll(".app-row").length>5', 90); time.sleep(2)
    scroll_through()
    installed = js(CARDS)
    with_logo = sum(c['logo'] for c in installed)
    note('installed', {'apps': len(installed), 'with_logo': with_logo, 'neutral_icon': sum(c['fallback'] for c in installed)})
    assert with_logo >= len(installed) * 0.75
    assert js('return [...document.querySelectorAll(".app-row button")].some(b=>b.textContent.includes(arguments[0]))', [L['remove']])
    opens = js('return [...document.querySelectorAll(".app-row .btn-open")].map(b=>b.getAttribute("aria-label"))')
    assert opens and all(label.startswith(L['open']) for label in opens), 'installed apps must have the Open button'
    note('installed: Open button', {'apps_with_open': len(opens), 'examples': opens[:3]})
    shot('16-installed.png')

    # ---- Updates: one button checks every source. Its confirmation is opened and cancelled: the APT part
    # would ask for the administrator password, so this script never runs it.
    js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")==="' + L['updates'] + '").click()')
    wait('return !!document.querySelector(".toolbar")', 60)
    buttons = js('return [...document.querySelectorAll(".toolbar button")].map(b=>b.textContent.trim())')
    assert buttons == [L['check']], f'the Updates page must offer one check button, found {buttons}'
    shot('17-updates.png')
    click_text(L['check'], 'document.querySelector(".toolbar")')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 60)
    plan = js('return {title:document.querySelector(".confirm-title").textContent, badge:!!document.querySelector(".confirm-head .source-badge"), command:document.querySelector(".command-preview pre").textContent, button:document.querySelector("[data-testid=confirm-operation]").textContent, details:[...document.querySelectorAll(".plan-details li")].length}')
    lines = plan['command'].splitlines()
    assert plan['title'] == L['check'] and not plan['badge'] and plan['button'] == L['check'], plan
    # The APT lists are refreshed through PackageKit (no password) when the system has it, else with pkexec.
    assert any('PackageKit' in line and 'RefreshCache' in line for line in lines) or any('apt-get' in line and 'update' in line and 'pkexec' in line for line in lines), lines
    assert any('snap refresh --list' in line for line in lines), lines
    assert not any(word in plan['command'] for word in ('upgrade', 'install', 'remove', 'autoremove')), lines
    note('update check confirmation', {'commands': lines, 'details': plan['details']})
    shot('18-update-check.png')
    if not os.environ.get('NEXA_RUN_CHECK'):
        click_text(L['cancel'])
        assert not modal_open()
    else:
        # Optional: really run the check. It refreshes the APT lists through PackageKit, the Flatpak catalog
        # and the Snap list; nothing is installed. If PackageKit is missing or refuses, the system asks for the
        # administrator password and this waits for it.
        js('document.querySelector("[data-testid=confirm-operation]").click()')
        # While it runs the window shows Operations; a successful check then returns to Updates.
        wait('return document.querySelector(".nav-item.active")?.getAttribute("aria-label")===arguments[0]'.replace('arguments[0]', json.dumps(L['updates'])) + ' && !!document.querySelector(".toolbar")', 300)
        note('update check executed: back on the Updates page', {'notice': js('return document.querySelector(".alert-success")?.textContent')})
        shot('19-after-check.png')
        js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")===arguments[0]).click()', [L['operations']])
        wait('return !!document.querySelector(".activity-card")', 30)
        output = js('return document.querySelector(".activity-card pre").textContent')
        assert js('return !!document.querySelector(".activity-card.success")'), output
        assert all(f'== {name}: {L["done"]} ==' in output for name in ('APT', 'Flatpak', 'Snap')), output
        note('update check output', {'output': output[:500]})
    ok = True
finally:
    try:
        if session: req('DELETE', f'/session/{session}')
    except Exception: pass
    drv.terminate()
    (OUT / 'gui-check.json').write_text(json.dumps({'ok': ok, 'checks': results}, ensure_ascii=False, indent=1))
    print('RESULT ok =', ok)
