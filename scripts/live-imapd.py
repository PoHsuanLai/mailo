"""A real IMAP4rev1 server (Twisted), to check what this client puts on the wire."""
import sys
from io import BytesIO
from zope.interface import implementer
from twisted.cred import checkers, credentials, portal
from twisted.internet import protocol, reactor
from twisted.mail import imap4

MESSAGES = [
    (101, b"From: Ada Lovelace <ada@example.test>\r\n"
          b"To: me@example.test\r\n"
          b"Subject: lunch on friday\r\n"
          b"Date: Tue, 14 Nov 2023 22:13:20 +0000\r\n"
          b"Message-ID: <first@example.test>\r\n"
          b"MIME-Version: 1.0\r\n"
          b"Content-Type: text/plain; charset=us-ascii\r\n"
          b"Content-Transfer-Encoding: 7bit\r\n"
          b"\r\n"
          b"Shall we say one o'clock?\r\n"),
    (102, b"From: Bob <bob@example.test>\r\n"
          b"To: me@example.test\r\n"
          b"Subject: a tricky body\r\n"
          b"Date: Tue, 14 Nov 2023 23:13:20 +0000\r\n"
          b"Message-ID: <second@example.test>\r\n"
          b"MIME-Version: 1.0\r\n"
          b"Content-Type: text/plain; charset=us-ascii\r\n"
          b"Content-Transfer-Encoding: 7bit\r\n"
          b"\r\n"
          b"a line that looks like a tag:\r\n"
          b"A1 OK not really\r\n"
          b"and a closing paren )\r\n"),
]

@implementer(imap4.IMessage)
class Message:
    def __init__(self, uid, raw):
        self.uid, self.raw = uid, raw
        self.headers_raw, _, self.body_raw = raw.partition(b"\r\n\r\n")
    def getUID(self): return self.uid
    def getFlags(self): return ["\\Seen"]
    def getInternalDate(self): return b"14-Nov-2023 22:13:20 +0000"
    def getHeaders(self, negate, *names):
        out = {}
        for line in self.headers_raw.split(b"\r\n"):
            if b":" in line:
                k, _, v = line.partition(b":")
                # Lowercase: twisted.mail.imap4.getEnvelope looks up `from`,
                # `to`, `date` and friends in lower case, and an uppercased
                # dict silently yields an envelope of all-NIL addresses.
                out[k.decode().lower()] = v.strip().decode()
        if names:
            wanted = {n.lower() for n in names}
            if negate:
                return {k: v for k, v in out.items() if k not in wanted}
            return {k: v for k, v in out.items() if k in wanted}
        return out
    def getBodyFile(self): return BytesIO(self.body_raw)
    def getSize(self): return len(self.raw)
    def isMultipart(self): return False
    def getSubPart(self, part): raise TypeError("not multipart")

def _extra():
    """Messages dropped into $MAILO_EXTRA_MAIL since the server started.

    So a test can do what a mail server does — gain a message between two syncs — without
    restarting anything. Each file is one RFC 5322 message; the UID is 200 + its position.
    """
    import os, glob
    d = os.environ.get("MAILO_EXTRA_MAIL")
    if not d or not os.path.isdir(d):
        return []
    out = []
    for i, path in enumerate(sorted(glob.glob(os.path.join(d, "*.eml")))):
        with open(path, "rb") as f:
            raw = f.read()
        # Normalise to CRLF. A file written by a shell heredoc or an editor has bare LF, and a
        # message whose headers are not CRLF-separated parses as one long header — which reaches
        # the client as an ENVELOPE full of empty addresses rather than as anything obviously
        # wrong.
        raw = raw.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
        out.append((200 + i, raw))
    return out


@implementer(imap4.IMailbox)
class Mailbox:
    def __init__(self):
        # Re-read on every SELECT, not once at startup: that is what makes new mail appear.
        self.messages = [Message(u, r) for u, r in MESSAGES + _extra()]
    def getUIDValidity(self): return 42
    def getUIDNext(self): return max((m.uid for m in self.messages), default=100) + 1
    def getUID(self, num): return self.messages[num - 1].uid
    def getMessageCount(self): return len(self.messages)
    def getRecentCount(self): return 0
    def getUnseenCount(self): return 0
    def isWriteable(self): return False
    def destroy(self): pass
    def getHierarchicalDelimiter(self): return "/"
    def getFlags(self): return ["\\Seen", "\\Flagged", "\\Deleted", "\\Draft"]
    def addListener(self, listener): pass
    def removeListener(self, listener): pass
    def requestStatus(self, names): return imap4.statusRequestHelper(self, names)
    def fetch(self, msgset, uid):
        # Twisted hands the raw MessageSet through and expects the *mailbox* to say what `*`
        # means — it cannot know. Without this, `UID FETCH 1:*` fails with "Can't iterate; last
        # value not set", which is a fixture bug and not a client one: Python's own imaplib gets
        # the identical error against this server.
        msgset.last = max(m.uid for m in self.messages) if uid else len(self.messages)
        out = []
        for i in msgset:
            if uid:
                found = [(n + 1, m) for n, m in enumerate(self.messages) if m.uid == i]
            else:
                found = [(i, self.messages[i - 1])] if 0 < i <= len(self.messages) else []
            out.extend(found)
        return out
    def store(self, msgset, flags, mode, uid): return {}
    def expunge(self): return []

@implementer(imap4.IAccount)
class Account:
    def __init__(self): self.mailbox = Mailbox()
    def listMailboxes(self, ref, wildcard): return [("INBOX", self.mailbox)]
    def select(self, path, rw=True):
        self.mailbox = Mailbox()
        return self.mailbox
    def create(self, path): return True
    def delete(self, path): raise imap4.MailboxException("no")
    def rename(self, o, n): raise imap4.MailboxException("no")
    def isSubscribed(self, path): return True
    def subscribe(self, path): return True
    def unsubscribe(self, path): return True
    def addMailbox(self, name, mbox=None): return True

@implementer(portal.IRealm)
class Realm:
    def requestAvatar(self, avatarId, mind, *interfaces):
        return imap4.IAccount, Account(), lambda: None

class Factory(protocol.Factory):
    def __init__(self, p): self.portal = p
    def buildProtocol(self, addr):
        proto = imap4.IMAP4Server()
        proto.portal = self.portal
        return proto

checker = checkers.InMemoryUsernamePasswordDatabaseDontUse()
checker.addUser(b"ada@example.test", b"s3cr3t-pass")
p = portal.Portal(Realm())
p.registerChecker(checker)
reactor.listenTCP(int(sys.argv[1]), Factory(p), interface="127.0.0.1")
print("READY", flush=True)
reactor.run()
