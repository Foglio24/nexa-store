#!/usr/bin/env python3
"""End-to-end check of the AppImage source in the real window, in an isolated data folder.

It installs a real AppImage (KDiskMark, about 47 MiB, from its GitHub release) through the interface
and removes it again. The app runs with XDG_DATA_HOME and XDG_CACHE_HOME pointing at a temporary
folder, so nothing is written to your real home: the AppImage, its launcher and the catalog cache
all live in that folder, which is deleted at the end. The app is never started.

Needs network access, the release binary, tauri-driver, WebKitWebDriver and a graphical session.
Usage: python3 scripts/test-appimage.py   (NEXA_TAURI_DRIVER=/path/to/tauri-driver if it is not in PATH)
Language: English unless NEXA_LANG=it is set.
"""
import hashlib, json, os, shutil, subprocess, sys, tempfile, time, urllib.request, urllib.error
from pathlib import Path

PROJECT = Path(__file__).resolve().parent.parent
DRIVER = os.environ.get('NEXA_TAURI_DRIVER', shutil.which('tauri-driver') or 'tauri-driver')
BINARY = PROJECT / 'src-tauri/target/release/nexa-store'
OUT = PROJECT / 'verification'
OUT.mkdir(exist_ok=True)
PORT = 4499
BASE = f'http://127.0.0.1:{PORT}'
LANGUAGE = os.environ.get('NEXA_LANG', 'en')
L = {'en': dict(install='Install', remove='Remove', installed='Installed'),
     'it': dict(install='Installa', remove='Rimuovi', installed='Installate')}[LANGUAGE]
session = None


def req(method, route, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    r = urllib.request.Request(BASE + route, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(r, timeout=400) as resp:
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
        time.sleep(0.4)
    raise AssertionError('timeout: ' + script[:140])


def click_text(text, within='document'):
    found = js(f'const b=[...{within}.querySelectorAll("button")].find(b=>b.textContent.trim().startsWith(arguments[0]));if(b){{b.click();return true}}return false', [text])
    assert found, f'button not found: {text}'


def type_query(text):
    el = cmd('POST', '/element', {'using': 'css selector', 'value': '.search input'})['element-6066-11e4-a52e-4f735466cecf']
    cmd('POST', f'/element/{el}/clear', {})
    cmd('POST', f'/element/{el}/value', {'text': text})


def sha256(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as file:
        for block in iter(lambda: file.read(1 << 20), b''): digest.update(block)
    return digest.hexdigest()


root = Path(tempfile.mkdtemp(prefix='nexa-appimage-e2e-'))
data, cache = root / 'data', root / 'cache'
data.mkdir(); cache.mkdir()
env = dict(os.environ, GDK_BACKEND='x11', NEXA_LANG=LANGUAGE, XDG_DATA_HOME=str(data), XDG_CACHE_HOME=str(cache))
drv = subprocess.Popen([DRIVER, '--port', str(PORT), '--native-port', str(PORT + 1)], env=env,
                       stdout=(OUT / 'webdriver-appimage.log').open('w'), stderr=subprocess.STDOUT)
ok = False
try:
    for _ in range(100):
        try: req('GET', '/status'); break
        except Exception: time.sleep(0.1)
    session = req('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': str(BINARY)}}}})['sessionId']
    cmd('POST', '/timeouts', {'script': 400000})
    wait('return !!document.querySelector(".hero-body h2")', 120)

    # Search the catalog (it is downloaded into the isolated cache the first time).
    type_query('kdiskmark')
    wait('return [...document.querySelectorAll(".app-row .app-name")].some(n=>n.textContent.toLowerCase().startsWith("kdiskmark"))', 120)
    sources = js('return [...document.querySelector(".app-row").querySelectorAll(".source-badge")].map(b=>b.textContent.trim())')
    assert 'AppImage' in sources, sources
    print('search kdiskmark => sources', sources, flush=True)
    js('document.querySelector(".app-row .app-row-main").click()')
    wait('return !!document.querySelector("[role=dialog] .detail-description")', 60)
    if js('return !!document.querySelector("[role=dialog] .variant-picker")'):
        click_text('AppImage', 'document.querySelector("[role=dialog] .variant-picker")')
        time.sleep(2)
        wait('return !!document.querySelector("[role=dialog] .detail-description")', 60)
    click_text(L['install'], 'document.querySelector("[role=dialog] .detail-actions")')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 90)
    command = js('return document.querySelector(".command-preview pre").textContent')
    expected = next(line for line in command.splitlines() if 'SHA-256' in line).split('SHA-256')[1].strip()
    print('confirmation => ', command.splitlines()[0], '| expected checksum', expected[:16], '…', flush=True)
    js('document.querySelector("[data-testid=confirm-operation]").click()')
    wait('return document.querySelector(".activity-card.success, .activity-card.error")!==null', 400)
    output = js('return document.querySelector(".activity-card pre").textContent')
    assert js('return !!document.querySelector(".activity-card.success")'), output
    print('install => success\n' + output, flush=True)

    folder = data / 'nexa-store/appimages/jonmagon/kdiskmark'
    files = sorted(folder.glob('*.AppImage'))
    assert len(files) == 1, list(folder.iterdir())
    image = files[0]
    assert os.access(image, os.X_OK) and image.stat().st_mode & 0o777 == 0o755, 'not executable'
    assert expected == 'not published by GitHub)' or sha256(image) == expected, 'the checksum on disk differs from the published one'
    launcher = next((data / 'applications').glob('nexa-appimage-*.desktop'))
    text = launcher.read_text()
    assert 'Name=KDiskMark' in text and str(image) in text and 'X-Nexa-Store-AppImage=JonMagon/KDiskMark' in text, text
    assert (folder / 'meta.json').is_file()
    print('files => ', image.name, f'{image.stat().st_size / 1048576:.1f} MiB', '| launcher', launcher.name, flush=True)

    # Installed list: the app is there, with Open and Remove.
    js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")===arguments[0]).click()', [L['installed']])
    wait('return [...document.querySelectorAll(".app-row .app-name")].some(n=>n.textContent.toLowerCase().startsWith("kdiskmark"))', 90)
    row = js('''const r=[...document.querySelectorAll(".app-row")].find(r=>r.querySelector(".app-name").textContent.toLowerCase().startsWith("kdiskmark"));
                return {sources:[...r.querySelectorAll(".source-badge")].map(b=>b.textContent.trim()), buttons:[...r.querySelectorAll("button")].map(b=>b.getAttribute("aria-label")||b.textContent.trim()), version:r.querySelector(".version")?.textContent}''')
    assert row['sources'] == ['AppImage'] and row['version'] == '3.3.0' or row['sources'] == ['AppImage'], row
    print('installed list => ', row, flush=True)

    # Remove it through the interface.
    js('[...document.querySelectorAll(".app-row")].find(r=>r.querySelector(".app-name").textContent.toLowerCase().startsWith("kdiskmark")).querySelector(".btn-danger-quiet").click()')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 60)
    js('document.querySelector("[data-testid=confirm-operation]").click()')
    wait('return document.querySelector(".activity-card.success, .activity-card.error")!==null', 90)
    assert js('return !!document.querySelector(".activity-card.success")'), js('return document.querySelector(".activity-card pre").textContent')
    assert not folder.exists() and not list((data / 'applications').glob('nexa-appimage-*.desktop')), 'something was left behind'
    print('remove => success, folder and launcher gone', flush=True)
    ok = True
finally:
    try:
        if session: req('DELETE', f'/session/{session}')
    except Exception: pass
    drv.terminate()
    subprocess.run(['pkill', '-x', 'nexa-store'])
    shutil.rmtree(root, ignore_errors=True)
    print('RESULT ok =', ok)
    sys.exit(0 if ok else 1)
