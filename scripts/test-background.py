#!/usr/bin/env python3
"""Check that an operation keeps going with the window closed, and that notifications arrive.

It does not change the system: it uses the Snap update check (`snap refresh --list`, read-only),
which takes about half a second. The script starts it from the real interface and, while it runs,
closes the window with the same message the close button sends (WM_DELETE_WINDOW).

Expected: the window hides without stopping the operation, one "continues in the background"
notification, one end-of-operation notification that GNOME Shell does not delete when the app exits,
and the app quitting by itself. With no operation running, closing must end the app at once and
silently. With the window open, the end-of-operation notification must still be sent.

Real notifications appear on the desktop during the run. Needs Python 3, tauri-driver,
WebKitWebDriver, dbus-monitor, xprop, a graphical session, snapd and the release binary.
Usage: python3 scripts/test-background.py   (NEXA_TAURI_DRIVER=/path/to/tauri-driver if it is not in PATH)

The interface and the notifications are in Italian, so the strings checked below are Italian.
"""
import base64, ctypes, ctypes.util, json, os, shutil, subprocess, sys, threading, time, urllib.request, urllib.error
from pathlib import Path

PROJECT = Path(__file__).resolve().parent.parent
DRIVER = os.environ.get('NEXA_TAURI_DRIVER', shutil.which('tauri-driver') or 'tauri-driver')
BINARY = PROJECT / 'src-tauri/target/release/nexa-store'
OUT = PROJECT / 'verification'
OUT.mkdir(exist_ok=True)
PORT = 4477
BASE = f'http://127.0.0.1:{PORT}'
session = None
results = []


def req(method, route, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    r = urllib.request.Request(BASE + route, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(r, timeout=120) as resp:
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
    raise AssertionError('timeout: ' + script[:120])


def note(name, data):
    results.append({'check': name, 'result': data})
    print(name, '=>', json.dumps(data, ensure_ascii=False)[:300], flush=True)


def app_pids(): return subprocess.run(['pgrep', '-x', 'nexa-store'], capture_output=True, text=True).stdout.split()


def wait_exit(timeout):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if not app_pids(): return True
        time.sleep(0.1)
    return False


def window_id():
    ids = subprocess.run(['xprop', '-root', '_NET_CLIENT_LIST'], capture_output=True, text=True).stdout.split('#')[-1].replace(',', ' ').split()
    for wid in ids:
        if 'nexa' in subprocess.run(['xprop', '-id', wid, 'WM_CLASS'], capture_output=True, text=True).stdout.lower():
            return int(wid, 16)
    raise AssertionError('Nexa Store window not found')


class ClientMessage(ctypes.Structure):
    _fields_ = [('type', ctypes.c_int), ('serial', ctypes.c_ulong), ('send_event', ctypes.c_int), ('display', ctypes.c_void_p),
                ('window', ctypes.c_ulong), ('message_type', ctypes.c_ulong), ('format', ctypes.c_int), ('data', ctypes.c_long * 5),
                ('padding', ctypes.c_char * 64)]  # XEvent is 192 bytes: XSendEvent copies all of it


def close_window(wid):
    """Ask the window manager's way: the same WM_DELETE_WINDOW message the close button sends."""
    x11 = ctypes.CDLL(ctypes.util.find_library('X11'))
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XInternAtom.restype = ctypes.c_ulong
    x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x11.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    display = x11.XOpenDisplay(None)
    assert display, 'X11 display not reachable'
    event = ClientMessage()
    event.type = 33  # ClientMessage
    event.window = wid
    event.message_type = x11.XInternAtom(display, b'WM_PROTOCOLS', 0)
    event.format = 32
    event.data[0] = x11.XInternAtom(display, b'WM_DELETE_WINDOW', 0)
    event.data[1] = 0  # CurrentTime
    x11.XSendEvent(display, wid, 0, 0, ctypes.byref(event))
    x11.XFlush(display)


class NotificationWatch:
    """Notifications of Nexa Store and what the shell does with them, as seen on the session bus."""
    def __init__(self):
        self.proc = subprocess.Popen(['dbus-monitor',
                                      "type='method_call',interface='org.freedesktop.Notifications',member='Notify'",
                                      "type='method_return'",
                                      "type='signal',interface='org.freedesktop.Notifications',member='NotificationClosed'"],
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.lines = []
        threading.Thread(target=lambda: [self.lines.append(line.rstrip('\n')) for line in self.proc.stdout], daemon=True).start()
        time.sleep(1)

    def _blocks(self, since=0):
        block = None
        for line in self.lines[since:]:
            if line.startswith(('method call', 'method return', 'signal')):
                if block: yield block
                block = [line]
            elif block is not None and line.startswith(' '):
                block.append(line.strip())
        if block: yield block

    def events(self, since=0):
        """(summary -> notification id) and the list of (id, reason) the shell closed."""
        pending, ids, closed, order = {}, {}, [], []
        for block in self._blocks(since):
            head = block[0]
            sender = (head.split('sender=')[1].split()[0]) if 'sender=' in head else ''
            serial = head.split(' serial=')[1].split()[0].rstrip(';') if ' serial=' in head else ''
            if head.startswith('method call') and 'member=Notify' in head:
                strings = [item[8:-1] for item in block[1:] if item.startswith('string "')][:4]
                if len(strings) >= 3 and strings[0].startswith('Nexa Store'):
                    pending[(sender, serial)] = strings[2]
            elif head.startswith('method return') and 'reply_serial=' in head:
                destination = head.split('destination=')[1].split()[0]
                reply = head.split('reply_serial=')[1].split()[0]
                key = (destination, reply)
                if key in pending and len(block) > 1 and block[1].startswith('uint32'):
                    summary = pending[key]
                    ids[summary] = int(block[1].split()[1])
                    if summary not in order: order.append(summary)
            elif head.startswith('signal') and 'member=NotificationClosed' in head and len(block) > 2:
                closed.append((int(block[1].split()[1]), int(block[2].split()[1])))
        return order, ids, closed

    def summaries(self, since=0): return self.events(since)[0]
    def stop(self): self.proc.terminate()


drv = None


def start_driver():
    """One driver per scenario: when the app quits by itself the driver keeps the dead session."""
    global drv
    if drv: drv.terminate(); time.sleep(0.5)
    drv = subprocess.Popen([DRIVER, '--port', str(PORT), '--native-port', str(PORT + 1)], env=env,
                           stdout=(OUT / 'webdriver-background.log').open('a'), stderr=subprocess.STDOUT)
    for _ in range(100):
        try: req('GET', '/status'); return
        except Exception: time.sleep(0.1)


def start_session():
    global session
    start_driver()
    session = req('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': str(BINARY)}}}})['sessionId']
    cmd('POST', '/timeouts', {'script': 120000})
    wait('return !!document.querySelector(".hero-body h2")', 90)


env = dict(os.environ, GDK_BACKEND='x11')
watch = NotificationWatch()
ok = False
try:
    # ---- 1. Without a running operation, closing the window ends the app at once, silently.
    start_session()
    before = len(watch.summaries())
    close_window(window_id())
    assert wait_exit(8), 'with no operation, closing must end the app'
    time.sleep(0.5)
    assert len(watch.summaries()) == before, 'no notification expected on a normal close'
    note('close with no operation: the app quits, no notification', True)
    session = None

    # ---- 2. With an operation running, the window is hidden and the work finishes in background.
    since_scenario_2 = len(watch.lines)
    start_session()
    js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")==="Aggiornamenti").click()')
    wait('return !!document.querySelector(".toolbar")', 60)
    assert js('return [...document.querySelectorAll(".toolbar button")].some(b=>b.textContent.trim()==="Snap")'), 'snapd is not available'
    js('[...document.querySelectorAll(".toolbar button")].find(b=>b.textContent.trim()==="Snap").click()')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 60)
    wid = window_id()
    started = time.monotonic()
    js('document.querySelector("[data-testid=confirm-operation]").click()')
    time.sleep(0.15)  # the operation takes about half a second; it is running by now
    close_window(wid)
    time.sleep(0.2)
    alive_after_close = bool(app_pids())
    assert alive_after_close, 'the app quit during the operation'
    note('window closed during the operation: the app stays alive', {'after_ms': round((time.monotonic() - started) * 1000)})
    assert wait_exit(20), 'the app must quit by itself when the operation ends'
    note('the app quits by itself when the operation ends', {'after_s': round(time.monotonic() - started, 1)})
    time.sleep(7)  # GNOME Shell deletes a notification linked to the app about a second after the app leaves the bus
    summaries, ids, closed = watch.events(since_scenario_2)
    note('notifications sent to the desktop', summaries)
    assert 'Nexa Store continua in background' in summaries, summaries
    final = next((text for text in summaries if text.startswith('Controllo Snap') and 'terminato' in text), None)
    assert final, summaries
    closed_ids = [notification for notification, reason in closed]
    note('final notification after the app exits: the shell does not delete it', {'id': ids[final], 'closures_seen': closed})
    assert ids[final] not in closed_ids, 'the shell deleted the final notification when the app exited'

    # ---- 3. With the window open and in use the user must be told too: a silent install feels broken.
    start_session()
    mark = len(watch.lines)
    js('[...document.querySelectorAll(".nav-item")].find(b=>b.getAttribute("aria-label")==="Aggiornamenti").click()')
    wait('return !!document.querySelector(".toolbar")', 60)
    js('[...document.querySelectorAll(".toolbar button")].find(b=>b.textContent.trim()==="Snap").click()')
    wait('return document.querySelector("[data-testid=confirm-operation]")!==null', 60)
    js('document.querySelector("[data-testid=confirm-operation]").click()')
    wait('return document.querySelector(".activity-card.success, .activity-card.error")!==null', 60)
    assert js('return !!document.querySelector(".activity-card.success")'), 'Snap check failed'
    time.sleep(1.5)
    open_window = watch.summaries(since=mark)
    note('window open: the end-of-operation notification is still sent', open_window)
    assert any(text.startswith('Controllo Snap') and 'terminato' in text for text in open_window), open_window
    assert 'Nexa Store continua in background' not in open_window, 'no background notice with the window open'
    assert app_pids(), 'with the window open the app must stay open'
    ok = True
finally:
    watch.stop()
    try:
        if session: req('DELETE', f'/session/{session}')
    except Exception: pass
    if drv: drv.terminate()
    (OUT / 'background-check.json').write_text(json.dumps({'ok': ok, 'checks': results}, ensure_ascii=False, indent=1))
    print('RESULT ok =', ok)
