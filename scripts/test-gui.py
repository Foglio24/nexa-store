#!/usr/bin/env python3
"""UI check of Nexa Store on the real window.

It installs, removes and updates nothing: it searches, browses categories, opens details and
confirmations (then cancels them) and only runs `snap refresh --list`, which lists updates without
applying them. Needs the release binary, tauri-driver, WebKitWebDriver and a graphical session.
Usage: python3 scripts/test-gui.py   (NEXA_TAURI_DRIVER=/path/to/tauri-driver if it is not in PATH)

The interface is in Italian, so the selectors below use the Italian labels.
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
        js('document.querySelector("button[aria-label=\\"Cancella ricerca\\"]")?.click()')
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


env = dict(os.environ, GDK_BACKEND='x11')
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
    wait('return document.querySelectorAll(".dot.on").length>=3')
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
    click_text('Mostra altre app')
    time.sleep(1)
    wait('return !document.querySelector("[aria-busy=true]")', 60); time.sleep(2)
    after = js('return [...document.querySelectorAll(".grid-cards")[0].querySelectorAll(".app-name")].map(x=>x.textContent)')
    assert before != after, 'the proposals do not change'
    note('show more apps: different proposals', {'before': before[:4], 'after': after[:4]})

    # ---- A category page
    js('[...document.querySelectorAll(".category-tile")].find(t=>t.textContent.trim()==="Giochi").click()')
    wait('return document.querySelector("h1")?.textContent==="Giochi"')
    settle(24)
    scroll_through()
    games = js(CARDS)
    count = js('return document.querySelector(".result-count")?.textContent')
    assert len(games) >= 24 and all(not c['tech'] for c in games)
    note('category Games', {'cards': len(games), 'total': count, 'with_logo': sum(c['logo'] for c in games), 'sources': sorted({s for c in games for s in c['sources']})})
    assert sum(c['logo'] for c in games) >= len(games) * 0.9
    shot('11-category-games.png')
    click_text('Snap', 'document.querySelector(".segmented[aria-label=Sorgente]")')
    time.sleep(1); settle(12)
    only_snap = js(CARDS)
    assert only_snap and all(c['sources'] == ['Snap'] for c in only_snap), 'Snap filter not respected'
    note('category Games, Snap only', {'cards': len(only_snap), 'examples': [c['name'] for c in only_snap[:4]]})
    click_text('Tutte', 'document.querySelector(".segmented[aria-label=Sorgente]")')
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
    click_text('Pacchetti', 'document.querySelector(".segmented[aria-label=Tipo]")'); settle(3)
    technical = js(CARDS)
    assert any(r['name'] == 'gimp-data' for r in technical) and all(r['tech'] for r in technical)
    assert all(r['fallback'] or r['logo'] for r in technical)
    note('search gimp (technical packages)', [r['name'] for r in technical[:5]])
    shot('13-search-packages.png')
    click_text('Tutto', 'document.querySelector(".segmented[aria-label=Tipo]")'); settle(3)
    click_text('App', 'document.querySelector(".segmented[aria-label=Tipo]")'); settle(3)

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
    click_text('Installa', 'document.querySelector("[role=dialog] .detail-actions")')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 90)
    plan = js('return {title:document.querySelector(".confirm-title").textContent, source:document.querySelector(".confirm-head .source-badge")?.textContent.trim(), command:document.querySelector(".command-preview pre").textContent, button:document.querySelector("[data-testid=confirm-operation]").textContent}')
    assert plan['title'].startswith('Installa') and plan['source'] == 'Flatpak' and 'flatpak' in plan['command']
    note('install confirmation (not executed)', {'title': plan['title'], 'source': plan['source'], 'button': plan['button']})
    shot('15-confirmation.png')
    click_text('Annulla')
    assert not modal_open()
    type_query('')

    # ---- Installed: real logos
    js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")==="Installate").click()')
    wait('return document.querySelectorAll(".app-row").length>5', 90); time.sleep(2)
    scroll_through()
    installed = js(CARDS)
    with_logo = sum(c['logo'] for c in installed)
    note('installed', {'apps': len(installed), 'with_logo': with_logo, 'neutral_icon': sum(c['fallback'] for c in installed)})
    assert with_logo >= len(installed) * 0.75
    assert js('return [...document.querySelectorAll(".app-row button")].some(b=>b.textContent.includes("Rimuovi"))')
    opens = js('return [...document.querySelectorAll(".app-row .btn-open")].map(b=>b.getAttribute("aria-label"))')
    assert opens and all(label.startswith('Apri ') for label in opens), 'installed apps must have the Open button'
    note('installed: Open button', {'apps_with_open': len(opens), 'examples': opens[:3]})
    shot('16-installed.png')

    # ---- Updates: the read-only Snap check, all the way to the Operations page
    js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")==="Aggiornamenti").click()')
    wait('return !!document.querySelector(".toolbar")', 60)
    shot('17-updates.png')
    if js('return [...document.querySelectorAll(".toolbar button")].some(b=>b.textContent.trim()==="Snap")'):
        click_text('Snap', 'document.querySelector(".toolbar")')
        wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 60)
        command = js('return document.querySelector(".command-preview pre").textContent')
        assert 'snap refresh --list' in command and 'pkexec' not in command, command
        js('document.querySelector("[data-testid=confirm-operation]").click()')
        wait('return document.querySelector(".activity-card.success, .activity-card.error")!==null', 120)
        assert js('return !!document.querySelector(".activity-card.success")'), 'Snap check failed'
        note('Snap update check executed (read-only)', {'command': command, 'output': js('return document.querySelector(".activity-card pre").textContent')[:160]})
        shot('18-operations.png')
    ok = True
finally:
    try:
        if session: req('DELETE', f'/session/{session}')
    except Exception: pass
    drv.terminate()
    (OUT / 'gui-check.json').write_text(json.dumps({'ok': ok, 'checks': results}, ensure_ascii=False, indent=1))
    print('RESULT ok =', ok)
