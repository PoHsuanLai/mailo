#!/usr/bin/env python3
"""Phase-0 spike: dump raw Gmail IMAP wire bytes.

Not production code. Deleted after phase 0. Captures the real wire format so
mail-proto's trace fixtures and mail-domain's RemoteRef/AccountCaps/SyncCursor
are designed against observed bytes rather than recalled RFCs.

Usage:
    IMAP_USER=you@gmail.com IMAP_PASS='app password' python3 spike/imap_dump.py

Gmail app password: myaccount.google.com/apppasswords (requires 2-Step Verification).
Output: spike/out/imap.trace  (password never written)
"""
import os, re, ssl, socket, sys, pathlib

HOST, PORT = "imap.gmail.com", 993
USER = os.environ.get("IMAP_USER")
PASS = os.environ.get("IMAP_PASS")
if not USER or not PASS:
    sys.exit("set IMAP_USER and IMAP_PASS")

OUT = pathlib.Path(__file__).parent / "out"
OUT.mkdir(exist_ok=True)
log = open(OUT / "imap.trace", "wb")

sock = ssl.create_default_context().wrap_socket(
    socket.create_connection((HOST, PORT), timeout=30), server_hostname=HOST)
sock.settimeout(10)

def record(direction, data):
    log.write(b"\n=== " + direction + b" ===\n" + data)
    log.flush()

def drain(tag):
    """Read until the tagged completion line, or timeout."""
    buf = b""
    pat = re.compile(rb"^" + re.escape(tag) + rb" (OK|NO|BAD)", re.M)
    while True:
        try:
            chunk = sock.recv(65536)
        except socket.timeout:
            break
        if not chunk:
            break
        buf += chunk
        if pat.search(buf):
            break
    record(b"S->C", buf)
    return buf.decode("utf-8", "replace")

# greeting
try:
    record(b"S->C greeting", sock.recv(65536))
except socket.timeout:
    pass

n = [0]
def cmd(line, secret=False, shown=None):
    n[0] += 1
    tag = f"a{n[0]:03d}".encode()
    wire = tag + b" " + line.encode() + b"\r\n"
    record(b"C->S", (tag + b" " + (shown or line).encode() + b"\r\n") if secret else wire)
    sock.sendall(wire)
    return drain(tag)

# Gmail advertises a REDUCED capability list before authentication -- no CONDSTORE, no
# MOVE, and XLIST in place of SPECIAL-USE. Reading caps pre-auth reports a server far
# less capable than it is, so ask again afterwards and believe the second answer.
pre_auth = cmd("CAPABILITY")
cmd(f'LOGIN "{USER}" "{PASS}"', secret=True, shown=f'LOGIN "{USER}" "<REDACTED>"')
caps = cmd("CAPABILITY")
list_raw = cmd('LIST "" "*"')
cmd('LIST (SPECIAL-USE) "" "*"')

FETCH_ITEMS = "(UID FLAGS INTERNALDATE ENVELOPE BODYSTRUCTURE X-GM-MSGID X-GM-THRID X-GM-LABELS)"

def sample(mailbox):
    """EXAMINE a mailbox and fetch its last 5 messages. Returns {msgid: uid}."""
    r = cmd(f'EXAMINE "{mailbox}"')
    m = re.search(r"\*\s+(\d+)\s+EXISTS", r)
    if not m or int(m.group(1)) == 0:
        return {}, r
    hi = int(m.group(1)); lo = max(1, hi - 4)
    f = cmd(f"FETCH {lo}:{hi} {FETCH_ITEMS}")
    pairs = {}
    for line in f.splitlines():
        u = re.search(r"UID (\d+)", line)
        g = re.search(r"X-GM-MSGID (\d+)", line)
        if u and g:
            pairs[g.group(1)] = u.group(1)
    return pairs, r

inbox, inbox_raw = sample("INBOX")
allmail, _ = sample("[Gmail]/All Mail")
cmd("LOGOUT")
log.close()

def has(x):
    return "yes" if x.upper() in caps.upper() else "NO"

print("\n--- spike summary -------------------------------------------")
print(f"  caps read POST-AUTH; pre-auth list was {'identical' if pre_auth == caps else 'SMALLER (expected)'}")
print(f"  CONDSTORE     {has('CONDSTORE')}")
print(f"  QRESYNC       {has('QRESYNC')}")
print(f"  MOVE          {has('MOVE')}")
print(f"  X-GM-EXT-1    {has('X-GM-EXT-1')}")
print(f"  IDLE          {has('IDLE')}")
print(f"  SPECIAL-USE   {has('SPECIAL-USE')}  (XLIST: {has('XLIST')})")
# LIST attributes are ground truth whatever CAPABILITY claims.
roles = sorted(set(re.findall(r"\\(All|Sent|Drafts|Trash|Junk|Flagged|Important)", list_raw)))
print(f"  LIST special-use attributes seen: {roles or 'none'}")
utf7 = [n for n in re.findall(r'\* LIST \([^)]*\) "[^"]*" "([^"]*)"', list_raw) if "&" in n]
print(f"  folders needing modified-UTF7 decoding: {len(utf7)}")
uv = re.search(r"UIDVALIDITY (\d+)", inbox_raw)
un = re.search(r"UIDNEXT (\d+)", inbox_raw)
hm = re.search(r"HIGHESTMODSEQ (\d+)", inbox_raw)
print(f"  INBOX         UIDVALIDITY={uv and uv.group(1)} UIDNEXT={un and un.group(1)} "
      f"HIGHESTMODSEQ={hm and hm.group(1)}")

shared = set(inbox) & set(allmail)
print(f"\n  THE KEY EXPERIMENT -- same message, two mailboxes, two UIDs:")
if not shared:
    print("    (no overlap in the last 5 of each; try a larger sample)")
for msgid in sorted(shared):
    same = "SAME  <-- unexpected" if inbox[msgid] == allmail[msgid] else "DIFFERENT"
    print(f"    X-GM-MSGID {msgid}:  INBOX uid={inbox[msgid]:>8}  "
          f"All Mail uid={allmail[msgid]:>8}   {same}")
print(f"\n  raw trace -> {OUT / 'imap.trace'}")
print("  NOTE: contains subjects and addresses from your mail. Scrub before committing.")
