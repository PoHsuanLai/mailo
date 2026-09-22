#!/usr/bin/env python3
"""Phase-0 spike: dump raw POP3 wire bytes from a real server.

Usage:
    POP_HOST=pop.example.edu POP_USER=<local-part> POP_PASS='...' python3 spike/pop3_dump.py

POP_HOST is required; POP_PORT defaults to 995 (implicit TLS).
Output: spike/out/pop3.trace  (password never written)
"""
import os, re, ssl, socket, sys, pathlib

HOST = os.environ.get("POP_HOST")
PORT = int(os.environ.get("POP_PORT", "995"))
USER = os.environ.get("POP_USER")
PASS = os.environ.get("POP_PASS")
if not HOST or not USER or not PASS:
    sys.exit("set POP_HOST, POP_USER and POP_PASS")

OUT = pathlib.Path(__file__).parent / "out"
OUT.mkdir(exist_ok=True)
log = open(OUT / "pop3.trace", "wb")

ctx = ssl.create_default_context()
try:
    sock = ctx.wrap_socket(socket.create_connection((HOST, PORT), timeout=30),
                           server_hostname=HOST)
except ssl.SSLCertVerificationError as e:
    print(f"!! cert verification FAILED: {e}")
    print("!! This is a real finding -- record it. Retrying unverified to continue the spike.")
    log.write(f"\n=== CERT VERIFICATION FAILED: {e} ===\n".encode())
    ctx = ssl._create_unverified_context()
    sock = ctx.wrap_socket(socket.create_connection((HOST, PORT), timeout=30),
                           server_hostname=HOST)
    print(f"!! peer cert subject: {sock.getpeercert()}")
sock.settimeout(10)

def record(d, data):
    log.write(b"\n=== " + d + b" ===\n" + data); log.flush()

def drain(multiline=False):
    buf = b""
    while True:
        try:
            chunk = sock.recv(65536)
        except socket.timeout:
            break
        if not chunk:
            break
        buf += chunk
        if not multiline and b"\r\n" in buf:
            break
        if multiline and (buf.endswith(b"\r\n.\r\n") or buf.startswith(b"-ERR")):
            break
    record(b"S->C", buf)
    return buf.decode("utf-8", "replace")

record(b"S->C greeting", sock.recv(65536))

def cmd(line, multiline=False, secret=False, shown=None):
    record(b"C->S", ((shown or line) + "\r\n").encode())
    sock.sendall((line + "\r\n").encode())
    return drain(multiline)

capa = cmd("CAPA", multiline=True)
cmd(f"USER {USER}")
cmd(f"PASS {PASS}", secret=True, shown="PASS <REDACTED>")
stat = cmd("STAT")
uidl = cmd("UIDL", multiline=True)
cmd_list = cmd("LIST", multiline=True)
cmd("RETR 1", multiline=True)
cmd("QUIT")
log.close()

print("\n--- spike summary -------------------------------------------")
print(f"  STAT          {stat.strip()}")
print(f"  UIDL support  {'yes' if not uidl.startswith('-ERR') else 'NO -- blocker'}")
# TOP and PIPELINING decide whether a large first sync is minutes or seconds, so they
# matter more than the auth mechanisms. The first version of this script omitted both.
for mech in ("TOP", "UIDL", "PIPELINING", "RESP-CODES", "SASL", "PLAIN", "LOGIN",
             "CRAM-MD5", "STLS", "USER"):
    print(f"  CAPA {mech:<12} {'yes' if mech in capa.upper() else 'no'}")
sizes = sorted(int(m.group(2)) for m in re.finditer(r"^(\\d+) (\\d+)\\r?$", cmd_list, re.M))
if sizes:
    total = sum(sizes)
    small = [x for x in sizes if x <= 64 * 1024]
    print(f"\n  size distribution ({len(sizes)} messages, {total/2**20:.1f} MiB):")
    print(f"    <=64KiB: {len(small)} msgs ({100*len(small)/len(sizes):.0f}%) "
          f"= {100*sum(small)/total:.0f}% of bytes")
    print(f"    top 100: {100*sum(sizes[-100:])/total:.0f}% of bytes")
    print(f"    median={sizes[len(sizes)//2]:,}  max={sizes[-1]:,}")
print(f"\n  UIDL shape (first lines):")
for line in uidl.splitlines()[:4]:
    print(f"    {line}")
print(f"\n  raw trace -> {OUT / 'pop3.trace'}")
print("  NOTE: RETR 1 dumps a full message. Scrub before committing.")
