// Tiny Markdown renderer — enough for our changelog files.
// Supports: ATX headings, paragraphs, `inline code`, **bold**, *italic*,
// links, unordered lists (with nested), fenced code blocks (```lang ... ```),
// and horizontal rules.
// Deliberately strict: a complex document will still render, but we don't
// try to match every CommonMark edge case.

(function () {
  "use strict";

  const esc = (s) =>
    s.replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");

  function inline(text) {
    // Protect inline code and spoiler placeholders so later replacements
    // don't touch them. Placeholder markers use control bytes so real text
    // can't collide.
    const codes = [];
    text = text.replace(/`([^`]+)`/g, (_, c) => {
      codes.push(`<code class="notranslate" translate="no">${esc(c)}</code>`);
      return `\0${codes.length - 1}\0`;
    });

    const spoilers = [];
    text = text.replace(
      /<span class="spoiler-locked" data-reveal-at="([^"]+)"><\/span>/g,
      (_, ts) => {
        spoilers.push(
          `<span class="spoiler-locked" data-reveal-at="${esc(ts)}" translate="no"></span>`
        );
        return `\x01${spoilers.length - 1}\x01`;
      }
    );

    text = esc(text);

    // Links: [label](url)
    text = text.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, label, url) => {
      const safeUrl = /^(https?:|#|\.\/|\/|mailto:)/i.test(url) ? url : "#";
      return `<a href="${safeUrl}" rel="noopener noreferrer">${label}</a>`;
    });

    // **bold**
    text = text.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
    // *italic* (skip content that was already converted to bold)
    text = text.replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<em>$2</em>");

    // Restore protected spans.
    text = text.replace(/\0(\d+)\0/g, (_, i) => codes[+i]);
    text = text.replace(/\x01(\d+)\x01/g, (_, i) => spoilers[+i]);
    return text;
  }

  function render(md) {
    const lines = md.replace(/\r\n?/g, "\n").split("\n");
    let out = "";
    let i = 0;

    // Track open list stacks: array of indent widths.
    let listStack = [];
    const closeListsTo = (depth) => {
      while (listStack.length > depth) {
        out += "</li></ul>";
        listStack.pop();
      }
    };

    while (i < lines.length) {
      const line = lines[i];

      // Fenced code block
      const fence = line.match(/^```(\w*)\s*$/);
      if (fence) {
        closeListsTo(0);
        i++;
        let body = "";
        while (i < lines.length && !/^```\s*$/.test(lines[i])) {
          body += lines[i] + "\n";
          i++;
        }
        if (i < lines.length) i++; // consume closing fence
        const lang = fence[1] ? ` lang-${fence[1]}` : "";
        out += `<pre class="notranslate${lang}" translate="no"><code>${esc(body.replace(/\n$/, ""))}</code></pre>`;
        continue;
      }

      // Heading
      const h = line.match(/^(#{1,6})\s+(.+?)\s*#*\s*$/);
      if (h) {
        closeListsTo(0);
        const level = h[1].length;
        out += `<h${level}>${inline(h[2])}</h${level}>`;
        i++;
        continue;
      }

      // Horizontal rule
      if (/^\s*---+\s*$/.test(line)) {
        closeListsTo(0);
        out += "<hr />";
        i++;
        continue;
      }

      // Blockquote
      if (/^>\s?/.test(line)) {
        closeListsTo(0);
        let quote = "";
        while (i < lines.length && /^>\s?/.test(lines[i])) {
          quote += lines[i].replace(/^>\s?/, "") + "\n";
          i++;
        }
        out += `<blockquote><p>${inline(quote.trim())}</p></blockquote>`;
        continue;
      }

      // Unordered list
      const li = line.match(/^(\s*)[-*+]\s+(.*)$/);
      if (li) {
        const indent = li[1].length;
        let content = li[2];
        // Depth: each 2-space bump is one level.
        const depth = Math.floor(indent / 2) + 1;
        if (depth > listStack.length) {
          while (depth > listStack.length) {
            out += "<ul><li>";
            listStack.push(depth);
          }
        } else {
          while (depth < listStack.length) {
            out += "</li></ul>";
            listStack.pop();
          }
          out += "</li><li>";
        }
        // Fold soft-wrapped continuation lines (indented further than the
        // marker) into this list item.
        i++;
        while (
          i < lines.length &&
          !/^\s*$/.test(lines[i]) &&
          !/^\s*[-*+]\s+/.test(lines[i]) &&
          /^\s+\S/.test(lines[i])
        ) {
          content += " " + lines[i].trim();
          i++;
        }
        out += inline(content);
        continue;
      }

      // Blank line terminates paragraph / list.
      if (/^\s*$/.test(line)) {
        closeListsTo(0);
        i++;
        continue;
      }

      // Paragraph: gather until blank line or block-starter.
      closeListsTo(0);
      let para = line;
      i++;
      while (
        i < lines.length &&
        !/^\s*$/.test(lines[i]) &&
        !/^#{1,6}\s/.test(lines[i]) &&
        !/^```/.test(lines[i]) &&
        !/^\s*[-*+]\s+/.test(lines[i]) &&
        !/^>\s?/.test(lines[i]) &&
        !/^\s*---+\s*$/.test(lines[i])
      ) {
        para += "\n" + lines[i];
        i++;
      }
      out += `<p>${inline(para)}</p>`;
    }

    closeListsTo(0);
    return out;
  }

  /**
   * Parse a document with optional YAML frontmatter between `---` fences.
   * Returns { data, body } where `data` is a flat string->string map.
   * Values surrounded by `[ ... ]` are split on commas into an array.
   */
  function parseFrontmatter(src) {
    const m = src.match(/^---\s*\n([\s\S]*?)\n---\s*\n?([\s\S]*)$/);
    if (!m) return { data: {}, body: src };
    const data = {};
    for (const raw of m[1].split(/\n/)) {
      const mm = raw.match(/^([A-Za-z0-9_-]+)\s*:\s*(.*)$/);
      if (!mm) continue;
      let v = mm[2].trim();
      // strip matching surrounding quotes
      if ((v.startsWith('"') && v.endsWith('"')) ||
          (v.startsWith("'") && v.endsWith("'"))) {
        v = v.slice(1, -1);
      }
      if (v.startsWith("[") && v.endsWith("]")) {
        data[mm[1]] = v.slice(1, -1)
          .split(",")
          .map((s) => s.trim().replace(/^["']|["']$/g, ""))
          .filter(Boolean);
      } else {
        data[mm[1]] = v;
      }
    }
    return { data, body: m[2] };
  }

  window.MD = { render, parseFrontmatter };
})();
