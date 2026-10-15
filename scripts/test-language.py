#!/usr/bin/env python3
"""Check that the real app follows the system language: Italian for Italian, English for the rest.

It starts the release binary several times with different locale variables (never NEXA_LANG) and
reads what the interface shows: the language of the page and the label of the first sidebar entry.
It changes nothing on the system. Needs the release binary, tauri-driver, WebKitWebDriver and a
graphical session.
Usage: python3 scripts/test-language.py   (NEXA_TAURI_DRIVER=/path/to/tauri-driver if it is not in PATH)
"""
import json, os, shutil, subprocess, sys, time, urllib.request, urllib.error
from pathlib import Path

PROJECT = Path(__file__).resolve().parent.parent
DRIVER = os.environ.get('NEXA_TAURI_DRIVER', shutil.which('tauri-driver') or 'tauri-driver')
BINARY = PROJECT / 'src-tauri/target/release/nexa-store'
OUT = PROJECT / 'verification'
OUT.mkdir(exist_ok=True)
PORT = 4488
BASE = f'http://127.0.0.1:{PORT}'

# (locale variables, expected language). The list mirrors how gettext resolves LANGUAGE and LC_*.
CASES = [
    ({'LANG': 'it_IT.UTF-8'}, 'it'),
    ({'LANG': 'it_IT.UTF-8', 'LANGUAGE': 'it'}, 'it'),
    ({'LANG': 'en_US.UTF-8'}, 'en'),
    ({'LANG': 'en_GB.UTF-8'}, 'en'),
    ({'LANG': 'de_DE.UTF-8'}, 'en'),
    ({'LANG': 'fr_FR.UTF-8'}, 'en'),
    ({'LANG': 'ja_JP.UTF-8'}, 'en'),
    ({'LANG': 'es_ES.UTF-8', 'LANGUAGE': 'es:it:en'}, 'it'),
    ({'LANG': 'it_IT.UTF-8', 'LC_MESSAGES': 'en_US.UTF-8'}, 'en'),
    ({'LANG': 'C'}, 'en'),
]
EXPLORE = {'it': 'Esplora', 'en': 'Explore'}


def req(method, route, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    r = urllib.request.Request(BASE + route, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(r, timeout=60) as resp:
            body = json.load(resp)
    except urllib.error.HTTPError as e:
        raise RuntimeError(e.read().decode()) from e
    return body.get('value', body)


def run_case(variables):
    env = {k: v for k, v in os.environ.items() if not k.startswith(('LC_', 'NEXA_')) and k not in ('LANG', 'LANGUAGE')}
    env.update(variables, GDK_BACKEND='x11')
    driver = subprocess.Popen([DRIVER, '--port', str(PORT), '--native-port', str(PORT + 1)], env=env,
                              stdout=(OUT / 'webdriver-language.log').open('a'), stderr=subprocess.STDOUT)
    session = None
    try:
        for _ in range(100):
            try: req('GET', '/status'); break
            except Exception: time.sleep(0.1)
        session = req('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': str(BINARY)}}}})['sessionId']
        script = 'return {lang: document.documentElement.lang, nav: document.querySelector(".nav-item span")?.textContent}'
        for _ in range(100):
            value = req('POST', f'/session/{session}/execute/sync', {'script': script, 'args': []})
            if value.get('nav'): return value
            time.sleep(0.3)
        raise AssertionError('the interface did not appear')
    finally:
        try:
            if session: req('DELETE', f'/session/{session}')
        except Exception: pass
        driver.terminate()
        driver.wait()
        subprocess.run(['pkill', '-x', 'nexa-store'])
        time.sleep(0.5)


ok = True
for variables, expected in CASES:
    try:
        seen = run_case(variables)
        good = seen['lang'] == expected and seen['nav'] == EXPLORE[expected]
    except Exception as error:
        seen, good = str(error), False
    ok &= good
    print('OK ' if good else 'FAIL', variables, '->', expected, '| seen:', seen, flush=True)
print('RESULT ok =', ok)
sys.exit(0 if ok else 1)
