"""A real POP3 server (Twisted), for running the journey a POP3 account will take.

    ./venv/bin/python scripts/live-pop3d.py 11110
    # then point an account's plan at 127.0.0.1:11110 with tls=plaintext
"""
import sys
from io import BytesIO
from zope.interface import implementer
from twisted.cred import checkers, portal
from twisted.internet import protocol, reactor
from twisted.mail import pop3

# Line endings are bare `\n` on purpose. `twisted.mail.pop3` splits a message on newlines and
# terminates each line it sends with CRLF, so a fixture written with `\r\n` goes onto the wire
# as `\r\r\n` — which a client then stores faithfully, because that is what arrived. Checked
# against the socket before blaming anything on this side.
MESSAGES = [
    b"From: Ada Lovelace <ada@example.test>\n"
    b"To: me@example.test\n"
    b"Subject: lunch on friday\n"
    b"Date: Tue, 14 Nov 2023 22:13:20 +0000\n"
    b"Message-ID: <pop-first@example.test>\n"
    b"\n"
    b"Shall we say one o'clock?\n",
    # A body whose lines need dot-stuffing on the way out, and 8-bit bytes that must survive.
    b"From: Bob <bob@example.test>\n"
    b"To: me@example.test\n"
    b"Subject: dotted and eight-bit\n"
    b"Date: Tue, 14 Nov 2023 23:13:20 +0000\n"
    b"Message-ID: <pop-second@example.test>\n"
    b"Content-Type: text/plain; charset=iso-8859-1\n"
    b"Content-Transfer-Encoding: 8bit\n"
    b"\n"
    b".a line beginning with a dot\n"
    b"caf\xe9 and na\xefve\n"
    b"and the end.\n",
]

@implementer(pop3.IMailbox)
class Mailbox:
    def __init__(self):
        self.deleted = set()
    def listMessages(self, index=None):
        sizes = [0 if i in self.deleted else len(m) for i, m in enumerate(MESSAGES)]
        return sizes[index] if index is not None else sizes
    def getMessage(self, index):
        return BytesIO(MESSAGES[index])
    def getUidl(self, index):
        return f"uidl-{index}".encode()
    def deleteMessage(self, index):
        self.deleted.add(index)
    def undeleteMessages(self):
        self.deleted.clear()
    def sync(self):
        pass

@implementer(portal.IRealm)
class Realm:
    def requestAvatar(self, avatarId, mind, *interfaces):
        return pop3.IMailbox, Mailbox(), lambda: None

class Factory(protocol.Factory):
    def __init__(self, p):
        self.portal = p
    def buildProtocol(self, addr):
        proto = pop3.POP3()
        proto.portal = self.portal
        return proto

checker = checkers.InMemoryUsernamePasswordDatabaseDontUse()
checker.addUser(b"ada@example.test", b"s3cr3t-pass")
p = portal.Portal(Realm())
p.registerChecker(checker)
reactor.listenTCP(int(sys.argv[1]), Factory(p), interface="127.0.0.1")
print("READY", flush=True)
reactor.run()
