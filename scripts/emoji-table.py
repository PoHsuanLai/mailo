#!/usr/bin/env python3
"""Write the composer's emoji table, crates/mail-app/src/emoji/table.txt.

Inputs, both Unicode data under the Unicode License v3 (SPDX Unicode-3.0, allowed by deny.toml):

  emoji-test.txt   https://www.unicode.org/Public/emoji/latest/emoji-test.txt
                   the groups, the order and the CLDR short name of every emoji
  annotations.json https://raw.githubusercontent.com/unicode-org/cldr-json/main/cldr-json/
                   cldr-annotations-full/annotations/en/annotations.json
                   CLDR's English keywords

Usage: scripts/emoji-table.py emoji-test.txt annotations.json > crates/mail-app/src/emoji/table.txt

Kept: fully-qualified emoji up to Emoji 15.0 (what a system colour emoji font can be expected to
draw), without skin-tone sequences and without the Component group. A keyword already a word of
the name is left out, since the name is searched too.
"""

import json
import re
import sys

MAX_VERSION = (15, 0)
SKIN = {0x1F3FB, 0x1F3FC, 0x1F3FD, 0x1F3FE, 0x1F3FF}

NOTICE = """\
# The emoji the composer offers: a table derived from Unicode data, written by
# scripts/emoji-table.py. Do not edit by hand.
#
# Sources: emoji-test.txt (Unicode Emoji {version}) for the groups, the order and the names,
# and CLDR's English annotations for the keywords.
#
# Format: "= Group" starts a group; then one emoji per line, tab-separated:
# the emoji, its CLDR short name, and keywords separated by spaces.
#
# UNICODE LICENSE V3
#
# COPYRIGHT AND PERMISSION NOTICE
#
# Copyright © 1991-2025 Unicode, Inc.
#
# NOTICE TO USER: Carefully read the following legal agreement. BY
# DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
# SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
# TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
# DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.
#
# Permission is hereby granted, free of charge, to any person obtaining a
# copy of data files and any associated documentation (the "Data Files") or
# software and any associated documentation (the "Software") to deal in the
# Data Files or Software without restriction, including without limitation
# the rights to use, copy, modify, merge, publish, distribute, and/or sell
# copies of the Data Files or Software, and to permit persons to whom the
# Data Files or Software are furnished to do so, provided that either (a)
# this copyright and permission notice appear with all copies of the Data
# Files or Software, or (b) this copyright and permission notice appear in
# associated Documentation.
#
# THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
# KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
# MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
# THIRD PARTY RIGHTS.
#
# IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
# BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
# OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
# WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
# ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
# FILES OR SOFTWARE.
#
# Except as contained in this notice, the name of a copyright holder shall
# not be used in advertising or otherwise to promote the sale, use or other
# dealings in these Data Files or Software without prior written
# authorization of the copyright holder.
#
# SPDX-License-Identifier: Unicode-3.0
"""

LINE = re.compile(
    r"^(?P<points>[0-9A-F ]+?)\s*;\s*(?P<status>[a-z-]+)\s*#\s*\S+\s+E(?P<major>\d+)\.(?P<minor>\d+)\s+(?P<name>.+)$"
)


def words(text):
    return [word for word in re.split(r"[\s:,]+", text.lower()) if word]


def main(test_path, annotations_path):
    with open(annotations_path, encoding="utf-8") as handle:
        annotations = json.load(handle)["annotations"]["annotations"]
    version = "?"
    group = None
    out = []
    with open(test_path, encoding="utf-8") as handle:
        for raw in handle:
            line = raw.rstrip("\n")
            if line.startswith("# Version:"):
                version = line.split(":", 1)[1].strip()
                continue
            if line.startswith("# group:"):
                group = line.split(":", 1)[1].strip()
                if group != "Component":
                    out.append(f"= {group}")
                continue
            if not line or line.startswith("#") or group == "Component":
                continue
            found = LINE.match(line)
            if not found or found["status"] != "fully-qualified":
                continue
            if (int(found["major"]), int(found["minor"])) > MAX_VERSION:
                continue
            points = [int(point, 16) for point in found["points"].split()]
            if SKIN & set(points):
                continue
            glyph = "".join(chr(point) for point in points)
            name = found["name"].strip()
            bare = glyph.replace("️", "")
            entry = annotations.get(glyph) or annotations.get(bare) or {}
            named = set(words(name))
            keywords = []
            for keyword in entry.get("default", []):
                for word in words(keyword):
                    if word not in named and word not in keywords:
                        keywords.append(word)
            out.append("\t".join([glyph, name, " ".join(keywords)]).rstrip())
    sys.stdout.write(NOTICE.format(version=version))
    sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
