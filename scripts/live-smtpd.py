"""A real SMTP server, for checking what this client actually puts on the wire.

The fake server in `submission_end_to_end.rs` is ours, so it agrees with our reading of the
protocol by construction — which is exactly the thing that can be wrong. `aiosmtpd` was written
by people who were not thinking about this client.

    python3 -m venv venv && ./venv/bin/pip install aiosmtpd
    ./venv/bin/python scripts/live-smtpd.py 12525 /tmp/received.eml
    cargo test -p mail-runtime --test live_smtp -- --ignored --nocapture

Then read /tmp/received.eml with `cat -A`: dot-stuffing bugs are visible there and nowhere else.
"""
import asyncio, sys
from aiosmtpd.controller import Controller
from aiosmtpd.smtp import AuthResult, LoginPassword

RECEIVED = sys.argv[2] if len(sys.argv) > 2 else "/tmp/received.eml"

def authenticator(server, session, envelope, mechanism, auth_data):
    # Exactly what a real server checks: the decoded username and password.
    if isinstance(auth_data, LoginPassword):
        if auth_data.login == b"ada@example.test" and auth_data.password == b"s3cr3t-pass":
            return AuthResult(success=True)
    return AuthResult(success=False, handled=False)

class Handler:
    async def handle_DATA(self, server, session, envelope):
        with open(RECEIVED, "wb") as f:
            f.write(b"MAIL FROM:" + envelope.mail_from.encode() + b"\n")
            f.write(b"RCPT TO:" + ",".join(envelope.rcpt_tos).encode() + b"\n")
            f.write(b"---\n")
            f.write(envelope.content)
        return "250 Message accepted"

controller = Controller(
    Handler(), hostname="127.0.0.1", port=int(sys.argv[1]),
    authenticator=authenticator, auth_require_tls=False,
)
controller.start()
print("READY", flush=True)
import time
try:
    while True:
        time.sleep(3600)
except KeyboardInterrupt:
    controller.stop()
