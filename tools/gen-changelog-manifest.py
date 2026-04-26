#!/usr/bin/env python3
"""Build the public changelog manifest with time-gated publishing + spoilers.

Reads changelog/*.md (source of truth). For each file:

  - publishAt > now   -> emit a placeholder manifest entry (build + publishAt),
                         DO NOT copy the file to --out-dir.
  - publishAt <= now  -> strip any ||spoiler|| blocks whose revealAt is still
                         in the future, then copy the rewritten body to
                         --out-dir and emit a normal manifest entry.

  Spoiler syntax:
      ||revealAt=2026-05-10T21:00:00+09:00|| hidden text ||

  Before revealAt the whole block (including the revealAt attr) is replaced by
  <span class="spoiler-locked" data-reveal-at="..."></span> on the server side,
  so the plaintext never reaches the client.

Run as:
    tools/gen-changelog-manifest.py [--out-dir /var/www/personaos/changelog]

With no --out-dir: writes manifest.json next to the source files (legacy
behaviour for local preview) and does not touch any other location.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "changelog"
DRAFTS_DIR = SRC_DIR / "_drafts"

FRONTMATTER_RE = re.compile(r"^---\s*\n(.*?)\n---\s*\n?(.*)$", re.DOTALL)
SCALAR_RE = re.compile(r"^([A-Za-z0-9_-]+)\s*:\s*(.*)$")

# Matches ||...|| non-greedily. The revealAt= prefix is the magic that marks
# this as a time-gated spoiler (regular ||text|| is ignored).
SPOILER_RE = re.compile(
    r"\|\|\s*revealAt\s*=\s*(?P<ts>[^|]+?)\s*\|\|(?P<body>.*?)\|\|",
    re.DOTALL,
)


def parse_frontmatter(text: str) -> tuple[dict, str]:
    m = FRONTMATTER_RE.match(text)
    if not m:
        return {}, text
    data: dict = {}
    for line in m.group(1).splitlines():
        mm = SCALAR_RE.match(line)
        if not mm:
            continue
        key, val = mm.group(1), mm.group(2).strip()
        # strip matching quotes
        if (val.startswith('"') and val.endswith('"')) or (
            val.startswith("'") and val.endswith("'")
        ):
            val = val[1:-1]
        if val.startswith("[") and val.endswith("]"):
            data[key] = [
                s.strip().strip("'\"")
                for s in val[1:-1].split(",")
                if s.strip()
            ]
        else:
            data[key] = val
    return data, m.group(2)


def parse_ts(raw: str) -> dt.datetime | None:
    if not raw:
        return None
    raw = raw.strip()
    # Python 3.11+ handles "Z" suffix; normalize just in case.
    if raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        d = dt.datetime.fromisoformat(raw)
    except ValueError:
        return None
    if d.tzinfo is None:
        d = d.replace(tzinfo=dt.timezone.utc)
    return d


def redact_spoilers(body: str, now: dt.datetime) -> str:
    def repl(m: re.Match) -> str:
        reveal = parse_ts(m.group("ts"))
        if reveal is None or reveal <= now:
            # Reveal time has passed (or unparseable): strip the markers but
            # keep the inner text.
            return m.group("body")
        # Still locked: replace with an empty placeholder. Nothing about
        # `body` is emitted — it never reaches the client.
        iso = reveal.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")
        return f'<span class="spoiler-locked" data-reveal-at="{iso}"></span>'

    return SPOILER_RE.sub(repl, body)


def promote_ready_drafts(now: dt.datetime) -> list[Path]:
    """Move drafts from _drafts/ up to changelog/ once publishAt has passed.

    Drafts with no publishAt (or an unparseable one) are treated as "always
    draft" — they stay in _drafts/ until the author edits the frontmatter.
    Returns the list of source Paths that were promoted.
    """
    if not DRAFTS_DIR.is_dir():
        return []
    now_utc = now.astimezone(dt.timezone.utc)
    promoted: list[Path] = []
    for p in sorted(DRAFTS_DIR.glob("*.md")):
        raw = p.read_text(encoding="utf-8")
        data, _ = parse_frontmatter(raw)
        publish_at = parse_ts(data.get("publishAt", ""))
        if publish_at is None or publish_at > now_utc:
            continue
        dest = SRC_DIR / p.name
        if dest.exists():
            # Name collision — leave the draft in place and warn so humans
            # notice. Don't clobber an already-shipped file.
            print(
                f"WARNING: draft {p.name} would overwrite {dest.name}; "
                f"skipping promotion",
                file=sys.stderr,
            )
            continue
        p.replace(dest)
        promoted.append(dest)
        print(f"promoted draft → {dest.relative_to(ROOT)}")
    return promoted


def build_entry(path: Path, now: dt.datetime) -> tuple[dict, str | None]:
    """Return (manifest_entry, rewritten_body_or_None).

    rewritten_body is None when the build is not yet published.
    """
    raw = path.read_text(encoding="utf-8")
    data, body = parse_frontmatter(raw)

    publish_at = parse_ts(data.get("publishAt", ""))
    now_utc = now.astimezone(dt.timezone.utc)

    try:
        build = int(data.get("build", "0"))
    except ValueError:
        build = 0

    if publish_at is not None and publish_at > now_utc:
        # Not public yet. Emit a placeholder — no codename / summary / file.
        return (
            {
                "file": None,
                "build": build,
                "locked": True,
                "publishAt": publish_at.astimezone(dt.timezone.utc)
                .isoformat()
                .replace("+00:00", "Z"),
            },
            None,
        )

    # Published: strip locked spoilers, reassemble file with frontmatter.
    redacted_body = redact_spoilers(body, now_utc)
    # Rebuild the file so the on-disk public copy mirrors the server view.
    head = path.read_text(encoding="utf-8")
    m = FRONTMATTER_RE.match(head)
    if m:
        rebuilt = f"---\n{m.group(1)}\n---\n{redacted_body}"
    else:
        rebuilt = redacted_body

    return (
        {
            "file": path.name,
            "build": build,
            "codename": data.get("codename", ""),
            "version": data.get("version", ""),
            "date": data.get("date", ""),
            "summary": data.get("summary", ""),
        },
        rebuilt,
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--out-dir",
        type=Path,
        default=None,
        help="Public changelog directory. If set, write filtered .md files and "
        "manifest.json here. If omitted, only manifest.json next to sources.",
    )
    ap.add_argument(
        "--now",
        default=None,
        help="Override current time (ISO 8601). Testing only.",
    )
    ap.add_argument(
        "--published-list",
        type=Path,
        default=None,
        help="Write the list of currently-published changelog file paths "
        "(relative to repo root), one per line, to this file. Used by the "
        "sync script to decide what's safe to git-add.",
    )
    ap.add_argument(
        "--promoted-list",
        type=Path,
        default=None,
        help="Write the list of drafts promoted on this run (relative to "
        "repo root), one per line. Empty file if none. The sync script uses "
        "a non-empty file as the signal to auto-commit + push.",
    )
    args = ap.parse_args()

    now = parse_ts(args.now) if args.now else dt.datetime.now(dt.timezone.utc)
    if now is None:
        print(f"bad --now: {args.now!r}", file=sys.stderr)
        return 2

    # Promote drafts whose publishAt has arrived from _drafts/ into the main
    # directory. This is what "flip a file into the repo" looks like from
    # git's perspective — the draft directory is gitignored, so the promoted
    # file appears as a fresh addition on the next commit.
    promoted = promote_ready_drafts(now)

    src_files = sorted(p for p in SRC_DIR.glob("*.md") if p.name.lower() != "readme.md")
    draft_files = sorted(DRAFTS_DIR.glob("*.md")) if DRAFTS_DIR.is_dir() else []

    entries: list[dict] = []
    public_bodies: dict[str, str] = {}
    for p in list(src_files) + list(draft_files):
        entry, body = build_entry(p, now)
        entries.append(entry)
        if body is not None and entry.get("file"):
            public_bodies[entry["file"]] = body

    entries.sort(key=lambda e: e.get("build", 0), reverse=True)
    manifest = json.dumps(entries, indent=2, ensure_ascii=False) + "\n"

    # Emit the published-list file if requested. Paths are relative to the
    # repo root so the sync script can feed them straight into `git add`.
    if args.published_list is not None:
        lines = [
            str((SRC_DIR / name).relative_to(ROOT))
            for name in sorted(public_bodies)
        ]
        lines.append(str((SRC_DIR / "manifest.json").relative_to(ROOT)))
        args.published_list.write_text("\n".join(lines) + "\n", encoding="utf-8")

    if args.promoted_list is not None:
        promoted_paths = [str(p.relative_to(ROOT)) for p in promoted]
        content = "\n".join(promoted_paths)
        if content:
            content += "\n"
        args.promoted_list.write_text(content, encoding="utf-8")

    if args.out_dir is None:
        # Legacy mode: manifest only, next to sources.
        (SRC_DIR / "manifest.json").write_text(manifest, encoding="utf-8")
        print(f"wrote {SRC_DIR / 'manifest.json'}")
        return 0

    out_dir: Path = args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)

    # Write only the files that are currently public. Delete everything else
    # in out_dir (so a build that flips back to locked, or is deleted from
    # source, disappears from /var/www on the next run).
    wanted = set(public_bodies) | {"manifest.json"}
    for existing in out_dir.iterdir():
        if existing.is_file() and existing.name not in wanted:
            existing.unlink()

    for name, body in public_bodies.items():
        (out_dir / name).write_text(body, encoding="utf-8")

    (out_dir / "manifest.json").write_text(manifest, encoding="utf-8")

    # Keep the source-side manifest updated too, so local preview still works.
    (SRC_DIR / "manifest.json").write_text(manifest, encoding="utf-8")

    print(
        f"published {len(public_bodies)} / {len(entries)} builds "
        f"to {out_dir}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
