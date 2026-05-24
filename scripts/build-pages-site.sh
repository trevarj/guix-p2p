#!/usr/bin/env bash
set -eu

site_dir="${SITE_DIR:-site}"
report_src="${BENCHMARK_RESULTS_MD:-target/guix-p2p-bench/benchmark-results.md}"
csv_src="${BENCHMARK_RESULTS_CSV:-target/guix-p2p-bench/results.csv}"
history_src="${BENCHMARK_HISTORY_DIR:-}"
asset_version="$(git rev-parse --short HEAD 2>/dev/null || date +%s)"

mkdir -p "$site_dir"
mkdir -p "$site_dir/assets"
cp docs/assets/guix-p2p-wordmark.svg "$site_dir/assets/guix-p2p-wordmark.svg"

if [ -r "$report_src" ]; then
  cp "$report_src" "$site_dir/benchmark-results.md"
else
  cat > "$site_dir/benchmark-results.md" <<'MARKDOWN'
# Benchmark Results

No benchmark report has been published yet.
MARKDOWN
fi

if [ -r "$csv_src" ]; then
  cp "$csv_src" "$site_dir/results.csv"
else
  printf 'status\nno benchmark CSV has been published yet\n' > "$site_dir/results.csv"
fi

if [ -n "$history_src" ] && [ -d "$history_src" ]; then
  history_tmp="$site_dir/history.csv.tmp"
  history_out="$site_dir/history.csv"
  first=true
  : > "$history_tmp"
  while IFS= read -r csv; do
    run_id="${csv#"$history_src"/}"
    run_id="${run_id%%/*}"
    if [ "$first" = true ]; then
      header="$(head -n 1 "$csv")"
      printf 'run_id,%s\n' "$header" > "$history_tmp"
      first=false
    fi
    tail -n +2 "$csv" | sed "s/^/$run_id,/" >> "$history_tmp"
  done < <(find "$history_src" -name results.csv -type f | sort)
  if [ "$first" = false ]; then
    mv "$history_tmp" "$history_out"
  else
    rm -f "$history_tmp"
    printf 'status\nno benchmark history has been published yet\n' > "$history_out"
  fi
else
  printf 'status\nno benchmark history has been published yet\n' > "$site_dir/history.csv"
fi

cp docs/benchmarks.md "$site_dir/benchmark-methodology.md"
cp docs/tester-quickstart.md "$site_dir/tester-quickstart.md"
cp docs/troubleshooting.md "$site_dir/troubleshooting.md"
cp docs/connectivity.md "$site_dir/connectivity.md"
cp docs/deployment.md "$site_dir/deployment.md"
cp docs/configuration.md "$site_dir/configuration.md"
cp docs/bootstrap-node.md "$site_dir/bootstrap-node.md"
cp docs/release.md "$site_dir/release.md"
rm -f "$site_dir/agent-brief.html" "$site_dir/agent-brief.md"

cat > "$site_dir/styles.css" <<'CSS'
:root {
  color-scheme: light dark;
  --bg: #fbfaf4;
  --surface: #ffffff;
  --surface-muted: #f4f1e7;
  --text: #1b2522;
  --muted: #65716c;
  --border: #d8d1bd;
  --accent: #f2b400;
  --accent-strong: #9a6a00;
  --green: #2f7d57;
  --link: #1769aa;
  --code-bg: #1f2937;
  --code-text: #e5e7eb;
  --code-muted: #94a3b8;
  --code-keyword: #93c5fd;
  --code-string: #fde68a;
  --code-comment: #9ca3af;
  --code-symbol: #86efac;
  --code-number: #fca5a5;
  --shadow: 0 18px 42px rgb(56 45 14 / 10%);
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  line-height: 1.5;
  background: var(--bg);
  color: var(--text);
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #101512;
    --surface: #171d19;
    --surface-muted: #20271f;
    --text: #ece8d8;
    --muted: #a9b2a9;
    --border: #344035;
    --accent: #f2b400;
    --accent-strong: #ffd75c;
    --green: #6fca98;
    --link: #7db7f0;
    --code-bg: #0f172a;
    --code-text: #e2e8f0;
    --code-muted: #94a3b8;
    --code-keyword: #7dd3fc;
    --code-string: #fde68a;
    --code-comment: #94a3b8;
    --code-symbol: #86efac;
    --code-number: #fca5a5;
    --shadow: 0 18px 42px rgb(0 0 0 / 28%);
  }
}
* {
  box-sizing: border-box;
}
html {
  background:
    radial-gradient(circle at 15% -10%, color-mix(in srgb, var(--accent) 24%, transparent), transparent 32rem),
    linear-gradient(180deg, color-mix(in srgb, var(--green) 10%, transparent), transparent 22rem),
    var(--bg);
}
body {
  background: transparent;
  margin: 0;
}
a {
  color: var(--link);
}
.site-header {
  backdrop-filter: blur(18px);
  background: color-mix(in srgb, var(--bg) 86%, transparent);
  border-bottom: 1px solid var(--border);
  position: sticky;
  top: 0;
  z-index: 10;
}
.site-header-inner, main {
  margin: 0 auto;
  max-width: 1180px;
  padding: 0 20px;
}
.site-header-inner {
  align-items: center;
  display: flex;
  gap: 18px;
  justify-content: space-between;
  min-height: 64px;
}
.brand {
  align-items: center;
  color: inherit;
  display: inline-flex;
  font-weight: 700;
  gap: 10px;
  text-decoration: none;
}
.brand img {
  background: #fbfaf4;
  border: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  border-radius: 8px;
  box-shadow: 0 8px 20px rgb(56 45 14 / 10%);
  display: block;
  height: 36px;
  padding: 3px 8px;
  width: 144px;
}
nav {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
nav a {
  border-radius: 999px;
  color: inherit;
  font-size: 0.95rem;
  opacity: 0.78;
  padding: 5px 9px;
  text-decoration: none;
}
nav a[aria-current="page"] {
  background: color-mix(in srgb, var(--accent) 20%, transparent);
  opacity: 1;
  font-weight: 650;
}
nav a:hover {
  background: color-mix(in srgb, var(--surface-muted) 70%, transparent);
  opacity: 1;
}
main {
  padding-bottom: 48px;
  padding-top: 36px;
}
.hero {
  border-bottom: 1px solid var(--border);
  margin-bottom: 28px;
  padding-bottom: 34px;
}
h1, h2, h3 {
  line-height: 1.2;
}
h1 {
  font-size: clamp(2rem, 4vw, 3.2rem);
  margin: 0 0 12px;
}
h2 {
  margin-top: 34px;
}
.lead {
  font-size: 1.08rem;
  max-width: 780px;
}
.muted {
  color: var(--muted);
}
.actions, .doc-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
.note {
  background: color-mix(in srgb, var(--accent) 14%, var(--surface));
  border: 1px solid color-mix(in srgb, var(--accent-strong) 28%, var(--border));
  border-radius: 8px;
  max-width: 780px;
  padding: 12px 14px;
}
.button, .doc-link {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: inherit;
  display: inline-flex;
  font-weight: 650;
  box-shadow: var(--shadow);
  padding: 10px 14px;
  text-decoration: none;
}
.button:first-child {
  background: var(--accent);
  border-color: color-mix(in srgb, var(--accent-strong) 42%, var(--accent));
  color: #1d1600;
}
.doc-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
}
.doc-link {
  align-items: flex-start;
  flex-direction: column;
  min-height: 104px;
}
.doc-link span {
  color: var(--muted);
  font-weight: 400;
  margin-top: 6px;
}
.section-grid {
  display: grid;
  gap: 16px;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
}
.info-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: var(--shadow);
  min-width: 0;
  padding: 16px;
}
.info-card h3 {
  font-size: 1.05rem;
  margin: 0 0 8px;
}
.info-card p,
.info-card ul {
  margin-bottom: 0;
}
.info-card ul {
  padding-left: 20px;
}
.markdown {
  overflow-wrap: anywhere;
}
.markdown ol,
.markdown ul {
  padding-left: 1.4rem;
}
.markdown li + li {
  margin-top: 0.25rem;
}
.table-wrap {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: var(--shadow);
  margin: 16px 0 24px;
  overflow-x: auto;
}
table {
  border-collapse: collapse;
  min-width: 100%;
  width: max-content;
}
th, td {
  border-bottom: 1px solid var(--border);
  padding: 10px 12px;
  text-align: left;
  vertical-align: top;
  white-space: nowrap;
}
th {
  background: var(--surface-muted);
  font-weight: 700;
  position: sticky;
  top: 0;
  z-index: 1;
}
tbody tr:nth-child(even) td {
  background: color-mix(in srgb, var(--surface-muted) 52%, transparent);
}
code {
  background: var(--surface-muted);
  border-radius: 5px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.92em;
  padding: 0.12em 0.34em;
}
pre {
  background: var(--code-bg);
  border: 1px solid color-mix(in srgb, var(--code-muted) 22%, transparent);
  border-radius: 8px;
  box-shadow: 0 18px 42px rgb(15 23 42 / 14%);
  color: var(--code-text);
  line-height: 1.55;
  margin: 14px 0;
  overflow: auto;
  padding: 18px 20px;
  position: relative;
}
pre code {
  background: transparent;
  border-radius: 0;
  color: inherit;
  display: block;
  font-size: 0.9rem;
  padding: 0;
}
pre[data-language]::before {
  color: var(--code-muted);
  content: attr(data-language);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.72rem;
  font-weight: 700;
  letter-spacing: 0;
  position: absolute;
  right: 14px;
  text-transform: uppercase;
  top: 10px;
}
.tok-keyword {
  color: var(--code-keyword);
  font-weight: 650;
}
.tok-string {
  color: var(--code-string);
}
.tok-comment {
  color: var(--code-comment);
  font-style: italic;
}
.tok-symbol {
  color: var(--code-symbol);
}
.tok-number {
  color: var(--code-number);
}
.chart-grid {
  display: grid;
  gap: 16px;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  margin: 16px 0 24px;
}
.chart-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: var(--shadow);
  min-width: 0;
  padding: 16px;
}
.chart-card h3 {
  font-size: 1rem;
  margin: 0 0 4px;
}
.chart-card p {
  margin: 0 0 14px;
}
.chart-svg {
  display: block;
  height: auto;
  max-width: 100%;
  overflow: visible;
  width: 100%;
}
.chart-label {
  fill: var(--text);
  font-size: 16px;
  font-weight: 600;
}
.chart-muted {
  fill: var(--muted);
  font-size: 14px;
}
.chart-axis {
  stroke: var(--border);
  stroke-width: 1;
}
.chart-bar-http {
  background: var(--link);
  fill: var(--link);
}
.chart-bar-p2p-only {
  background: var(--green);
  fill: var(--green);
}
.chart-bar-p2p-first {
  background: var(--accent);
  fill: var(--accent);
}
.chart-bar-http-first {
  background: #0ea5e9;
  fill: #0ea5e9;
}
.chart-phase-seed {
  background: #6f8fc7;
  fill: #6f8fc7;
}
.chart-phase-prepare {
  background: #c77d50;
  fill: #c77d50;
}
.chart-phase-p2p {
  background: var(--green);
  fill: var(--green);
}
.chart-phase-provider {
  background: #8b75c9;
  fill: #8b75c9;
}
.chart-phase-daemon {
  background: #d4a51c;
  fill: #d4a51c;
}
.chart-phase-import {
  background: #5aa6a6;
  fill: #5aa6a6;
}
.chart-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 14px;
  margin-top: 10px;
}
.chart-legend span {
  align-items: center;
  color: var(--muted);
  display: inline-flex;
  font-size: 0.9rem;
  gap: 6px;
}
.chart-swatch {
  border-radius: 999px;
  display: inline-block;
  height: 10px;
  width: 10px;
}
@media (max-width: 640px) {
  .site-header-inner {
    align-items: flex-start;
    flex-direction: column;
    gap: 8px;
    padding-bottom: 14px;
    padding-top: 14px;
  }
}
CSS

cat > "$site_dir/markdown.js" <<'JS'
function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function renderInline(value) {
  return escapeHtml(value)
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<a href="$2">$1</a>')
    .replace(/`([^`]+)`/g, "<code>$1</code>");
}

function languageLabel(language) {
  const labels = {
    scheme: "Scheme",
    sh: "Shell",
    shell: "Shell",
    bash: "Shell",
    toml: "TOML",
    text: "Text",
  };
  return labels[language] || language || "Text";
}

function detectLanguage(code, hint) {
  if (hint) return hint.toLowerCase();
  const trimmed = code.trimStart();
  if (trimmed.startsWith("(use-modules") || trimmed.startsWith("(cons*") || trimmed.includes("(channel")) {
    return "scheme";
  }
  if (/^(guix|sudo|cargo|\.\/)/m.test(trimmed)) {
    return "sh";
  }
  if (/^[a-z0-9_-]+\s*=/im.test(trimmed)) {
    return "toml";
  }
  return "text";
}

function highlightScheme(code) {
  return escapeHtml(code)
    .replace(/(&quot;[^&]*?&quot;)/g, '<span class="tok-string">$1</span>')
    .replace(/\b(use-modules|services|modify-services|service|channel|name|url|branch|cons\*|guix-service-type|guix-p2p-enable-guix-daemon-extension)\b/g, '<span class="tok-keyword">$1</span>')
    .replace(/(%[a-z0-9-]+|'[a-z0-9-]+)/gi, '<span class="tok-symbol">$1</span>');
}

function highlightShell(code) {
  return escapeHtml(code)
    .replace(/(#.*)$/gm, '<span class="tok-comment">$1</span>')
    .replace(/(&quot;[^&]*?&quot;|'[^']*?')/g, '<span class="tok-string">$1</span>')
    .replace(/\b(guix|sudo|cargo|git|export|GUIX_EXTENSIONS_PATH)\b/g, '<span class="tok-keyword">$1</span>')
    .replace(/(\s|^)(--[a-z0-9-]+)/gi, '$1<span class="tok-symbol">$2</span>');
}

function highlightToml(code) {
  return escapeHtml(code)
    .replace(/(#.*)$/gm, '<span class="tok-comment">$1</span>')
    .replace(/(&quot;[^&]*?&quot;)/g, '<span class="tok-string">$1</span>')
    .replace(/^([a-z0-9_-]+)(\s*=)/gim, '<span class="tok-keyword">$1</span>$2')
    .replace(/\b(\d+)\b/g, '<span class="tok-number">$1</span>');
}

function renderCodeBlock(code, hint) {
  const language = detectLanguage(code, hint);
  const highlighted = language === "scheme"
    ? highlightScheme(code)
    : language === "sh" || language === "shell" || language === "bash"
      ? highlightShell(code)
      : language === "toml"
        ? highlightToml(code)
        : escapeHtml(code);
  return `<pre data-language="${languageLabel(language)}"><code class="language-${language}">${highlighted}</code></pre>`;
}

function normalizeLine(line) {
  return line.replace(/^- Date: (\d+) seconds since 1970-01-01 UTC$/, (_match, seconds) => {
    const date = new Date(Number(seconds) * 1000);
    if (Number.isNaN(date.getTime())) {
      return line;
    }
    return `- Date: ${date.toISOString().replace("T", " ").replace(".000Z", " UTC")}`;
  });
}

function splitTableRow(line) {
  return line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((cell) => cell.trim());
}

function isSeparatorRow(line) {
  return /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(line);
}

function renderMarkdown(markdown) {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n").map(normalizeLine);
  const html = [];
  let paragraph = [];
  let list = [];
  let orderedList = [];
  let inFence = false;
  let fence = [];
  let fenceLanguage = "";

  function flushParagraph() {
    if (paragraph.length === 0) return;
    html.push(`<p>${renderInline(paragraph.join(" "))}</p>`);
    paragraph = [];
  }

  function flushList() {
    if (list.length === 0) return;
    html.push(`<ul>${list.map((item) => `<li>${renderInline(item)}</li>`).join("")}</ul>`);
    list = [];
  }

  function flushOrderedList() {
    if (orderedList.length === 0) return;
    html.push(`<ol>${orderedList.map((item) => `<li>${renderInline(item)}</li>`).join("")}</ol>`);
    orderedList = [];
  }

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];

    if (line.startsWith("```")) {
      if (inFence) {
        html.push(renderCodeBlock(fence.join("\n"), fenceLanguage));
        fence = [];
        fenceLanguage = "";
        inFence = false;
      } else {
        flushParagraph();
        flushList();
        flushOrderedList();
        fenceLanguage = line.slice(3).trim().split(/\s+/)[0] || "";
        inFence = true;
      }
      continue;
    }
    if (inFence) {
      fence.push(line);
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      flushList();
      flushOrderedList();
      continue;
    }

    const tableNext = lines[i + 1] || "";
    if (line.includes("|") && isSeparatorRow(tableNext)) {
      flushParagraph();
      flushList();
      flushOrderedList();
      const headers = splitTableRow(line);
      i += 2;
      const rows = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim() !== "") {
        rows.push(splitTableRow(lines[i]));
        i += 1;
      }
      i -= 1;
      html.push(`<div class="table-wrap"><table><thead><tr>${headers.map((cell) => `<th>${renderInline(cell)}</th>`).join("")}</tr></thead><tbody>${rows.map((row) => `<tr>${row.map((cell) => `<td>${renderInline(cell)}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`);
      continue;
    }

    const heading = /^(#{1,3})\s+(.+)$/.exec(line);
    if (heading) {
      flushParagraph();
      flushList();
      flushOrderedList();
      const level = heading[1].length;
      html.push(`<h${level}>${renderInline(heading[2])}</h${level}>`);
      continue;
    }

    const item = /^-\s+(.+)$/.exec(line);
    if (item) {
      flushParagraph();
      flushOrderedList();
      list.push(item[1]);
      continue;
    }

    const orderedItem = /^\d+\.\s+(.+)$/.exec(line);
    if (orderedItem) {
      flushParagraph();
      flushList();
      orderedList.push(orderedItem[1]);
      continue;
    }

    paragraph.push(line.trim());
  }

  flushParagraph();
  flushList();
  flushOrderedList();
  return html.join("\n");
}
JS

cat > "$site_dir/charts.js" <<'JS'
function parseCsv(text) {
  const rows = [];
  let row = [];
  let value = "";
  let quoted = false;

  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    const next = text[i + 1];
    if (quoted) {
      if (char === '"' && next === '"') {
        value += '"';
        i += 1;
      } else if (char === '"') {
        quoted = false;
      } else {
        value += char;
      }
      continue;
    }
    if (char === '"') {
      quoted = true;
    } else if (char === ",") {
      row.push(value);
      value = "";
    } else if (char === "\n") {
      row.push(value);
      rows.push(row);
      row = [];
      value = "";
    } else if (char !== "\r") {
      value += char;
    }
  }
  if (value.length > 0 || row.length > 0) {
    row.push(value);
    rows.push(row);
  }

  const headers = rows.shift() || [];
  return rows
    .filter((cells) => cells.some((cell) => cell !== ""))
    .map((cells) => Object.fromEntries(headers.map((header, index) => [header, cells[index] || ""])));
}

function parseDurationMs(value) {
  const trimmed = String(value || "").trim();
  if (trimmed === "" || trimmed === "n/a") return null;
  if (/^\d+(\.\d+)?$/.test(trimmed)) return Number(trimmed);
  const match = /^(\d+(?:\.\d+)?)(ms|s)$/.exec(trimmed);
  if (!match) return null;
  return match[2] === "s" ? Number(match[1]) * 1000 : Number(match[1]);
}

function formatDuration(ms) {
  if (ms === null || !Number.isFinite(ms)) return "n/a";
  if (ms >= 1000) return `${(ms / 1000).toFixed(ms >= 10_000 ? 1 : 2)}s`;
  return `${Math.round(ms)}ms`;
}

function parseNumber(value) {
  const trimmed = String(value || "").trim();
  if (trimmed === "") return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

function formatBytes(bytes) {
  if (bytes === null || !Number.isFinite(bytes)) return "n/a";
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 1 : 2)} ${units[unit]}`;
}

function formatThroughput(bytesPerSecond) {
  if (bytesPerSecond === null || !Number.isFinite(bytesPerSecond)) return "n/a";
  return `${formatBytes(bytesPerSecond)}/s`;
}

function modeClass(mode) {
  return `chart-bar-${String(mode || "unknown").replaceAll("_", "-")}`;
}

function displayCase(row) {
  const pieces = [row.package, row.http_condition || row["HTTP condition"], row.mode || row.Mode].filter(Boolean);
  return pieces.join(" / ");
}

function rowsFromCsv(csvText) {
  return parseCsv(csvText)
    .map((row) => ({
      package: row.package,
      mode: row.mode,
      http_condition: row.http_condition,
      seed_count: row.seed_count,
      success: row.success === "true",
      elapsed_ms: parseDurationMs(row.elapsed_ms),
      seed_ms: parseDurationMs(row.seed_ms),
      prepare_ms: parseDurationMs(row.prepare_ms),
      p2p_start_ms: parseDurationMs(row.p2p_start_ms),
      provider_wait_ms: parseDurationMs(row.provider_wait_ms),
      daemon_start_ms: parseDurationMs(row.daemon_start_ms),
      import_ms: parseDurationMs(row.import_ms),
      total_ms: parseDurationMs(row.total_ms || row.elapsed_ms),
      system_build_public_paths: parseNumber(row.system_build_public_paths),
      system_build_missing_before: parseNumber(row.system_build_missing_before),
      system_build_verified: parseNumber(row.system_build_verified),
      system_build_nar_bytes: parseNumber(row.system_build_nar_bytes),
      system_build_throughput_bps: parseNumber(row.system_build_throughput_bps),
      run_id: row.run_id || "",
    }))
    .filter((row) => row.package && row.mode && row.elapsed_ms !== null);
}

function markdownTable(markdown, heading) {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const headingIndex = lines.findIndex((line) => line.trim() === `## ${heading}`);
  if (headingIndex < 0) return [];
  const tableStart = lines.findIndex((line, index) => index > headingIndex && line.trim().startsWith("|"));
  if (tableStart < 0) return [];
  const tableLines = [];
  for (let i = tableStart; i < lines.length && lines[i].trim().startsWith("|"); i += 1) {
    tableLines.push(lines[i]);
  }
  if (tableLines.length < 3) return [];
  const headers = splitTableRow(tableLines[0]);
  return tableLines.slice(2).map((line) => {
    const cells = splitTableRow(line);
    return Object.fromEntries(headers.map((header, index) => [header, cells[index] || ""]));
  });
}

function rowsFromMarkdown(markdown) {
  return markdownTable(markdown, "Runs")
    .map((row) => ({
      package: row.Package,
      mode: row.Mode,
      http_condition: row["HTTP condition"],
      seed_count: row.Seeds,
      success: row.Status === "ok",
      elapsed_ms: parseDurationMs(row.Elapsed),
      seed_ms: parseDurationMs(row.Seed),
      prepare_ms: parseDurationMs(row.Prepare),
      p2p_start_ms: parseDurationMs(row["P2P start"]),
      provider_wait_ms: parseDurationMs(row["Provider wait"]),
      daemon_start_ms: parseDurationMs(row["Daemon start"]),
      import_ms: parseDurationMs(row.Import),
      total_ms: parseDurationMs(row.Elapsed),
      system_build_nar_bytes: parseNumber(row.Payload),
      system_build_throughput_bps: null,
      run_id: "",
    }))
    .filter((row) => row.package && row.mode && row.elapsed_ms !== null);
}

function median(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  return sorted[Math.floor((sorted.length - 1) / 2)];
}

function summarizeRows(rows) {
  const groups = new Map();
  for (const row of rows.filter((item) => item.success)) {
    const key = [row.package, row.http_condition, row.mode, row.seed_count].join("\u001f");
    const current = groups.get(key) || { ...row, samples: [] };
    current.samples.push(row);
    groups.set(key, current);
  }
  return [...groups.values()].map((group) => ({
    ...group,
    elapsed_ms: median(group.samples.map((row) => row.elapsed_ms)),
    seed_ms: median(group.samples.map((row) => row.seed_ms)),
    prepare_ms: median(group.samples.map((row) => row.prepare_ms)),
    p2p_start_ms: median(group.samples.map((row) => row.p2p_start_ms)),
    provider_wait_ms: median(group.samples.map((row) => row.provider_wait_ms)),
    daemon_start_ms: median(group.samples.map((row) => row.daemon_start_ms)),
    import_ms: median(group.samples.map((row) => row.import_ms)),
    system_build_public_paths: median(group.samples.map((row) => row.system_build_public_paths)),
    system_build_missing_before: median(group.samples.map((row) => row.system_build_missing_before)),
    system_build_verified: median(group.samples.map((row) => row.system_build_verified)),
    system_build_nar_bytes: median(group.samples.map((row) => row.system_build_nar_bytes)),
    system_build_throughput_bps: median(group.samples.map((row) => row.system_build_throughput_bps)),
  })).filter((row) => row.elapsed_ms !== null);
}

function renderElapsedChart(rows) {
  const data = summarizeRows(rows).sort((a, b) => a.elapsed_ms - b.elapsed_ms);
  if (data.length === 0) return "<p class=\"muted\">No successful benchmark timings are available yet.</p>";
  const width = 1160;
  const rowHeight = 46;
  const labelWidth = 390;
  const chartWidth = width - labelWidth - 150;
  const height = 52 + data.length * rowHeight;
  const max = Math.max(...data.map((row) => row.elapsed_ms));
  const bars = data.map((row, index) => {
    const y = 36 + index * rowHeight;
    const barWidth = Math.max(2, (row.elapsed_ms / max) * chartWidth);
    return `<g>
      <text class="chart-label" x="0" y="${y + 18}">${escapeHtml(displayCase(row))}</text>
      <rect class="${modeClass(row.mode)}" x="${labelWidth}" y="${y}" width="${barWidth}" height="24" rx="5"></rect>
      <text class="chart-label" x="${labelWidth + barWidth + 12}" y="${y + 18}">${formatDuration(row.elapsed_ms)}</text>
    </g>`;
  }).join("");
  return `<svg class="chart-svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="Median elapsed benchmark time by case">
    <line class="chart-axis" x1="${labelWidth}" y1="24" x2="${labelWidth}" y2="${height - 10}"></line>
    ${bars}
  </svg>
  <div class="chart-legend">
    <span><i class="chart-swatch chart-bar-http"></i>HTTP</span>
    <span><i class="chart-swatch chart-bar-p2p-only"></i>P2P only</span>
    <span><i class="chart-swatch chart-bar-p2p-first"></i>P2P first</span>
    <span><i class="chart-swatch chart-bar-http-first"></i>HTTP first</span>
  </div>`;
}

function renderThroughputChart(rows) {
  const data = summarizeRows(rows)
    .filter((row) => row.system_build_throughput_bps !== null)
    .sort((a, b) => b.system_build_throughput_bps - a.system_build_throughput_bps);
  if (data.length === 0) return "<p class=\"muted\">No system-build payload throughput is available yet.</p>";
  const width = 1160;
  const rowHeight = 46;
  const labelWidth = 390;
  const chartWidth = width - labelWidth - 190;
  const height = 52 + data.length * rowHeight;
  const max = Math.max(...data.map((row) => row.system_build_throughput_bps));
  const bars = data.map((row, index) => {
    const y = 36 + index * rowHeight;
    const barWidth = Math.max(2, (row.system_build_throughput_bps / max) * chartWidth);
    return `<g>
      <text class="chart-label" x="0" y="${y + 18}">${escapeHtml(displayCase(row))}</text>
      <rect class="${modeClass(row.mode)}" x="${labelWidth}" y="${y}" width="${barWidth}" height="24" rx="5"></rect>
      <text class="chart-label" x="${labelWidth + barWidth + 12}" y="${y + 18}">${formatThroughput(row.system_build_throughput_bps)}</text>
    </g>`;
  }).join("");
  return `<svg class="chart-svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="System-build payload throughput by case">
    <line class="chart-axis" x1="${labelWidth}" y1="24" x2="${labelWidth}" y2="${height - 10}"></line>
    ${bars}
  </svg>`;
}

function summarizeHistory(rows) {
  const groups = new Map();
  for (const row of rows.filter((item) => item.success && item.run_id)) {
    const key = [row.run_id, row.package, row.http_condition, row.mode, row.seed_count].join("\u001f");
    const current = groups.get(key) || { ...row, samples: [] };
    current.samples.push(row);
    groups.set(key, current);
  }
  return [...groups.values()].map((group) => ({
    ...group,
    elapsed_ms: median(group.samples.map((row) => row.elapsed_ms)),
  })).filter((row) => row.elapsed_ms !== null);
}

function renderHistoryChart(rows) {
  const data = summarizeHistory(rows).sort((a, b) => Number(a.run_id) - Number(b.run_id));
  if (data.length === 0) return "<p class=\"muted\">No benchmark history is available yet.</p>";
  const runIds = [...new Set(data.map((row) => row.run_id))];
  const cases = [...new Set(data.map((row) => displayCase(row)))];
  const width = 1160;
  const height = 120 + cases.length * 34;
  const left = 390;
  const chartWidth = width - left - 80;
  const max = Math.max(...data.map((row) => row.elapsed_ms));
  const runX = (runId) => {
    const index = runIds.indexOf(runId);
    if (runIds.length === 1) return left + chartWidth;
    return left + (index / (runIds.length - 1)) * chartWidth;
  };
  const rowsSvg = cases.map((label, index) => {
    const y = 42 + index * 34;
    const points = data
      .filter((row) => displayCase(row) === label)
      .map((row) => {
        const x = runX(row.run_id);
        const radius = Math.max(3, (row.elapsed_ms / max) * 11);
        return `<circle class="${modeClass(row.mode)}" cx="${x}" cy="${y}" r="${radius}"><title>Run ${row.run_id}: ${formatDuration(row.elapsed_ms)}</title></circle>`;
      })
      .join("");
    return `<g>
      <text class="chart-label" x="0" y="${y + 5}">${escapeHtml(label)}</text>
      ${points}
    </g>`;
  }).join("");
  const ticks = runIds.map((runId) => {
    const x = runX(runId);
    return `<g><line class="chart-axis" x1="${x}" y1="22" x2="${x}" y2="${height - 34}"></line><text class="chart-muted" x="${x - 18}" y="${height - 10}">${escapeHtml(runId)}</text></g>`;
  }).join("");
  return `<svg class="chart-svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="Benchmark median elapsed time across workflow runs">
    ${ticks}
    ${rowsSvg}
  </svg>`;
}

function renderPhaseChart(rows) {
  const phases = [
    ["seed_ms", "Seed", "chart-phase-seed"],
    ["prepare_ms", "Prepare", "chart-phase-prepare"],
    ["p2p_start_ms", "P2P start", "chart-phase-p2p"],
    ["provider_wait_ms", "Provider wait", "chart-phase-provider"],
    ["daemon_start_ms", "Daemon", "chart-phase-daemon"],
    ["import_ms", "Import", "chart-phase-import"],
  ];
  const data = summarizeRows(rows)
    .filter((row) => phases.some(([field]) => row[field] !== null))
    .sort((a, b) => a.elapsed_ms - b.elapsed_ms);
  if (data.length === 0) return "<p class=\"muted\">No phase timing data is available yet.</p>";
  const width = 1160;
  const rowHeight = 50;
  const labelWidth = 390;
  const chartWidth = width - labelWidth - 120;
  const height = 52 + data.length * rowHeight;
  const max = Math.max(...data.map((row) => phases.reduce((sum, [field]) => sum + (row[field] || 0), 0)));
  const bars = data.map((row, index) => {
    let x = labelWidth;
    const y = 36 + index * rowHeight;
    const segments = phases.map(([field, label, cssClass]) => {
      const value = row[field] || 0;
      if (value <= 0) return "";
      const width = Math.max(2, (value / max) * chartWidth);
      const segment = `<rect class="${cssClass}" x="${x}" y="${y}" width="${width}" height="24" rx="4"><title>${label}: ${formatDuration(value)}</title></rect>`;
      x += width;
      return segment;
    }).join("");
    return `<g>
      <text class="chart-label" x="0" y="${y + 18}">${escapeHtml(displayCase(row))}</text>
      ${segments}
      <text class="chart-label" x="${x + 12}" y="${y + 18}">${formatDuration(row.elapsed_ms)}</text>
    </g>`;
  }).join("");
  return `<svg class="chart-svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="Benchmark phase timing breakdown">
    <line class="chart-axis" x1="${labelWidth}" y1="24" x2="${labelWidth}" y2="${height - 10}"></line>
    ${bars}
  </svg>
  <div class="chart-legend">
    ${phases.map(([_field, label, cssClass]) => `<span><i class="chart-swatch ${cssClass}"></i>${label}</span>`).join("")}
  </div>`;
}

async function loadBenchmarkCharts() {
  const elapsed = document.getElementById("elapsed-chart");
  const phases = document.getElementById("phase-chart");
  const throughput = document.getElementById("throughput-chart");
  const history = document.getElementById("history-chart");
  if (!elapsed || !phases || !throughput || !history) return;
  const [csvResponse, markdownResponse, historyResponse] = await Promise.all([
    fetch("results.csv", { cache: "no-store" }),
    fetch("benchmark-results.md", { cache: "no-store" }),
    fetch("history.csv", { cache: "no-store" }),
  ]);
  const csvText = csvResponse.ok ? await csvResponse.text() : "";
  const markdown = markdownResponse.ok ? await markdownResponse.text() : "";
  const historyText = historyResponse.ok ? await historyResponse.text() : "";
  const csvRows = rowsFromCsv(csvText);
  const rows = csvRows.length > 0 ? csvRows : rowsFromMarkdown(markdown);
  const historyRows = rowsFromCsv(historyText);
  elapsed.innerHTML = renderElapsedChart(rows);
  phases.innerHTML = renderPhaseChart(rows);
  throughput.innerHTML = renderThroughputChart(rows);
  history.innerHTML = renderHistoryChart(historyRows);
}
JS

cat > "$site_dir/doc-page.js" <<'JS'
async function loadMarkdownDocument(source, fallbackTitle) {
  const documentTitle = document.getElementById("document-title");
  const documentBody = document.getElementById("document-body");
  const rawLink = document.getElementById("raw-markdown-link");
  if (rawLink) {
    rawLink.href = source;
  }
  const response = await fetch(source, { cache: "no-store" });
  if (!response.ok) {
    documentTitle.textContent = fallbackTitle;
    documentBody.textContent = `Could not load ${source}.`;
    return;
  }
  const markdown = await response.text();
  const heading = /^#\s+(.+)$/m.exec(markdown);
  documentTitle.textContent = heading ? heading[1] : fallbackTitle;
  documentBody.classList.remove("muted");
  documentBody.innerHTML = renderMarkdown(markdown);
}
JS

cat > "$site_dir/index.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html" aria-current="page">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="#configure">Configure</a>
        <a href="#development">Develop</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1>P2P binary substitutes for GNU Guix</h1>
      <p class="lead muted">guix-p2p is a libp2p daemon and Guix substitute extension. It lets Guix machines discover peers through a Kademlia DHT, download NAR blocks from those peers, verify official Guix nar hashes, and fall back to configured HTTP substitutes when policy allows it.</p>
      <p class="actions">
        <a class="button" href="#getting-started">Get started</a>
        <a class="button" href="tester-quickstart.html">Tester quickstart</a>
        <a class="button" href="configuration.html">Configuration reference</a>
        <a class="button" href="deployment.html">Deployment guide</a>
        <a class="button" href="benchmarks.html">Benchmarks</a>
        <a class="button" href="https://codeberg.org/trevarj/guix-p2p">Source on Codeberg</a>
      </p>
      <p class="note muted">Proof-of-concept experiment: this project is being developed using Codex GPT-5.5 with maintainer guidance.</p>
    </section>

    <section>
      <h2>What It Does</h2>
      <div class="section-grid">
        <article class="info-card">
          <h3>Uses normal Guix commands</h3>
          <p class="muted">Keep running <code>guix build</code>, <code>guix package</code>, and <code>guix system reconfigure</code>. The extension intercepts the internal <code>guix substitute</code> protocol calls made by <code>guix-daemon</code>.</p>
        </article>
        <article class="info-card">
          <h3>Verifies official content</h3>
          <p class="muted">Peers serve NAR blocks, but downloaded content is accepted only after Guix nar hash verification against trusted narinfo metadata.</p>
        </article>
        <article class="info-card">
          <h3>Finds peers over libp2p</h3>
          <p class="muted">Nodes advertise providers in Kademlia, can use bootstrap peers, and persist learned reachable peers between daemon starts.</p>
        </article>
      </div>
    </section>

    <section id="getting-started">
      <h2>Get Started</h2>
      <p>Add the authenticated repository channel, pull it, enable the system service, then use normal Guix commands. The persistent service defaults to http-first policy, the project bootstrap node, and a loopback dashboard.</p>
      <pre data-language="Scheme"><code class="language-scheme"><span class="tok-keyword">(cons*</span>
 (channel
  (<span class="tok-keyword">name</span> <span class="tok-symbol">'guix-p2p</span>)
  (<span class="tok-keyword">url</span> <span class="tok-string">"https://codeberg.org/trevarj/guix-p2p"</span>)
  (<span class="tok-keyword">branch</span> <span class="tok-string">"master"</span>)
  (<span class="tok-keyword">introduction</span>
   (<span class="tok-keyword">make-channel-introduction</span>
    <span class="tok-string">"7d6c023e60cffde9cc63d7c4c2232a3f9c9fca1f"</span>
    (<span class="tok-keyword">openpgp-fingerprint</span>
     <span class="tok-string">"A6C2 0D0C 2AD8 38F9 4907  0EA3 A52D 6879 4EBE D758"</span>))))
 <span class="tok-symbol">%default-channels</span>)</code></pre>
      <p>The introduction starts at the first signed <code>guix-p2p</code> commit that contains <code>.guix-authorizations</code>, so Guix can authenticate later channel updates.</p>
      <p>Import the service module in your operating-system configuration and enable the daemon extension:</p>
      <pre data-language="Scheme"><code class="language-scheme">(<span class="tok-keyword">use-modules</span> (guix-p2p services))

(<span class="tok-keyword">services</span>
  (<span class="tok-keyword">modify-services</span>
      (cons (<span class="tok-keyword">service</span> guix-p2p-service-type
                     (guix-p2p-configuration
                      (dashboard? #t)))
            <span class="tok-symbol">%base-services</span>)
    (<span class="tok-keyword">guix-service-type</span> config =>
      (<span class="tok-keyword">guix-p2p-enable-guix-daemon-extension</span> config))))</code></pre>
      <p>Pull, reconfigure, restart both services, and check readiness:</p>
      <pre data-language="Shell"><code class="language-sh"><span class="tok-keyword">guix</span> pull
<span class="tok-keyword">sudo</span> <span class="tok-keyword">guix</span> system reconfigure /etc/config.scm
<span class="tok-keyword">sudo</span> herd restart guix-p2p
<span class="tok-keyword">sudo</span> herd restart guix-daemon
guix-p2p --doctor</code></pre>
      <p>Open <code>http://127.0.0.1:3030</code> for dashboard readiness, then follow the <a href="tester-quickstart.html">tester quickstart</a> for a real seed/fetch validation.</p>
    </section>

    <section id="configure">
      <h2>Configure</h2>
      <div class="section-grid">
        <article class="info-card">
          <h3>Runtime config</h3>
          <p class="muted">The daemon reads <code>~/.config/guix-p2p/config.toml</code>. Configure substitute URLs, bootstrap peers, external addresses, cache paths, provider thresholds, and dashboard settings there.</p>
        </article>
        <article class="info-card">
          <h3>Bootstrap peers</h3>
          <p class="muted">The Guix System service defaults to the project bootstrap node. Use full peer multiaddrs such as <code>/dns4/node.example.org/udp/6881/quic-v1/p2p/12D3KooW...</code> when overriding <code>bootstrap-peers</code>. Do not share <code>/ip4/0.0.0.0/...</code>; that is only a local bind address.</p>
          <pre data-language="TOML"><code class="language-toml">bootstrap_peers = <span class="tok-string">"/dns4/guix-p2p.trevs.site/tcp/443/p2p/12D3KooWDnvPgCuPTPaMbnbLpXP7kCxmXc9F7agJPuAJWXGoDNPT"</span></code></pre>
        </article>
        <article class="info-card">
          <h3>Dashboard</h3>
          <p class="muted">Enable the dashboard to inspect daemon state, recent substitute activity, provider discovery, and the full shareable peer address derived from <code>external_addresses</code> plus the node PeerId.</p>
        </article>
      </div>
    </section>

    <section>
      <h2>Use</h2>
      <div class="section-grid">
        <article class="info-card">
          <h3>System service</h3>
          <p class="muted">The Guix service starts <code>guix-p2p --daemon</code>, adds the package to the system profile, and prepends the extension directory to <code>GUIX_EXTENSIONS_PATH</code> without clobbering other extensions.</p>
        </article>
        <article class="info-card">
          <h3>Substitution policy</h3>
          <p class="muted">The daemon answers <code>have</code> and <code>info</code> only when trusted narinfo metadata exists and enough P2P providers are available. HTTP fallback remains policy-controlled.</p>
        </article>
        <article class="info-card">
          <h3>Benchmarks</h3>
          <p class="muted">Benchmark results are secondary project evidence. The benchmark page renders the latest CI report, CSV, charts, and workflow-run links.</p>
        </article>
      </div>
    </section>

    <section id="development">
      <h2>Development</h2>
      <div class="section-grid">
        <article class="info-card">
          <h3>Local shell</h3>
          <p class="muted">Use the manifest for the contributor shell with Rust, Cargo, C toolchain packages, certificates, and test helpers:</p>
          <pre data-language="Shell"><code class="language-sh"><span class="tok-keyword">guix</span> shell -m manifest.scm</code></pre>
        </article>
        <article class="info-card">
          <h3>Package shell</h3>
          <p class="muted">Use the local Guix package entrypoint when you want to test the packaged binary and extension exactly as the channel package builds them:</p>
          <pre data-language="Shell"><code class="language-sh"><span class="tok-keyword">guix</span> shell -f guix.scm</code></pre>
        </article>
        <article class="info-card">
          <h3>Checks</h3>
          <p class="muted">Inside the contributor shell, run focused Rust checks before sending changes:</p>
          <pre data-language="Shell"><code class="language-sh"><span class="tok-keyword">cargo</span> fmt
<span class="tok-keyword">cargo</span> clippy <span class="tok-symbol">--all-targets</span> <span class="tok-symbol">--all-features</span> <span class="tok-symbol">--</span> -D warnings
<span class="tok-keyword">cargo</span> test</code></pre>
        </article>
        <article class="info-card">
          <h3>E2E proof</h3>
          <p class="muted">The VM channel proof is the strict deployment check. Longer benchmarks should run in CI, not on developer workstations.</p>
          <pre data-language="Shell"><code class="language-sh"><span class="tok-keyword">cargo</span> run -p guix-p2p-e2e <span class="tok-symbol">--</span> vm channel-proof</code></pre>
        </article>
      </div>
    </section>

    <section>
      <h2>Reference</h2>
      <div class="doc-grid">
        <a class="doc-link" href="tester-quickstart.html">Tester quickstart<span>Known-tester onboarding, dashboard checks, and real seed/fetch validation.</span></a>
        <a class="doc-link" href="troubleshooting.html">Troubleshooting<span>Symptom-first checks for common tester failures.</span></a>
        <a class="doc-link" href="connectivity.html">Connectivity<span>Bootstrap, NAT, firewall, and dashboard network signals.</span></a>
        <a class="doc-link" href="configuration.html">Configuration<span>Runtime options, paths, and substitute settings.</span></a>
        <a class="doc-link" href="deployment.html">Deployment<span>Bootstrap node and deployment notes.</span></a>
        <a class="doc-link" href="bootstrap-node.html">Bootstrap node<span>Operator checklist, upgrade, restart, and incident checks.</span></a>
        <a class="doc-link" href="release.html">Release process<span>Versioning, tagging, source build, and binary release policy.</span></a>
        <a class="doc-link" href="benchmark-methodology.md">Benchmark methodology<span>How local and VM benchmark suites are run.</span></a>
        <a class="doc-link" href="benchmarks.html">Benchmark results<span>Rendered latest report, CSV, charts, and workflow-run links.</span></a>
      </div>
    </section>
  </main>
</body>
</html>
HTML

cat > "$site_dir/configuration.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p configuration</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="configuration.html" aria-current="page">Configuration</a>
        <a href="deployment.html">Deployment</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1 id="document-title">Configuration Reference</h1>
      <p class="lead muted">Runtime options, paths, CLI overrides, and substitute policy settings.</p>
      <p class="actions">
        <a id="raw-markdown-link" class="button" href="configuration.md">Raw Markdown</a>
      </p>
    </section>
    <section>
      <div id="document-body" class="markdown muted">Loading configuration.md...</div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="doc-page.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    loadMarkdownDocument("configuration.md", "Configuration Reference");
  </script>
</body>
</html>
HTML

cat > "$site_dir/troubleshooting.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p troubleshooting</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html" aria-current="page">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="configuration.html">Configuration</a>
        <a href="deployment.html">Deployment</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1 id="document-title">Troubleshooting</h1>
      <p class="lead muted">Symptom-first checks for common tester failures.</p>
      <p class="actions">
        <a id="raw-markdown-link" class="button" href="troubleshooting.md">Raw Markdown</a>
      </p>
    </section>
    <section>
      <div id="document-body" class="markdown muted">Loading troubleshooting.md...</div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="doc-page.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    loadMarkdownDocument("troubleshooting.md", "Troubleshooting");
  </script>
</body>
</html>
HTML

cat > "$site_dir/connectivity.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p connectivity</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html" aria-current="page">Connectivity</a>
        <a href="configuration.html">Configuration</a>
        <a href="deployment.html">Deployment</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1 id="document-title">Connectivity</h1>
      <p class="lead muted">Bootstrap, NAT, firewall, and dashboard network signals.</p>
      <p class="actions">
        <a id="raw-markdown-link" class="button" href="connectivity.md">Raw Markdown</a>
      </p>
    </section>
    <section>
      <div id="document-body" class="markdown muted">Loading connectivity.md...</div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="doc-page.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    loadMarkdownDocument("connectivity.md", "Connectivity");
  </script>
</body>
</html>
HTML

cat > "$site_dir/tester-quickstart.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p tester quickstart</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html" aria-current="page">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="configuration.html">Configuration</a>
        <a href="deployment.html">Deployment</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1 id="document-title">Tester Quickstart</h1>
      <p class="lead muted">Known-tester onboarding, dashboard checks, and real seed/fetch validation.</p>
      <p class="actions">
        <a id="raw-markdown-link" class="button" href="tester-quickstart.md">Raw Markdown</a>
      </p>
    </section>
    <section>
      <div id="document-body" class="markdown muted">Loading tester-quickstart.md...</div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="doc-page.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    loadMarkdownDocument("tester-quickstart.md", "Tester Quickstart");
  </script>
</body>
</html>
HTML

cat > "$site_dir/deployment.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p deployment</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="configuration.html">Configuration</a>
        <a href="deployment.html" aria-current="page">Deployment</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1 id="document-title">Deployment Guide</h1>
      <p class="lead muted">Guix channel setup, daemon operation, substitute extension flow, and E2E validation.</p>
      <p class="actions">
        <a id="raw-markdown-link" class="button" href="deployment.md">Raw Markdown</a>
      </p>
    </section>
    <section>
      <div id="document-body" class="markdown muted">Loading deployment.md...</div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="doc-page.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    loadMarkdownDocument("deployment.md", "Deployment Guide");
  </script>
</body>
</html>
HTML

cat > "$site_dir/bootstrap-node.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p bootstrap node</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="configuration.html">Configuration</a>
        <a href="deployment.html">Deployment</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1 id="document-title">Bootstrap Node Guide</h1>
      <p class="lead muted">Operator checklist, upgrade, restart, and incident checks.</p>
      <p class="actions">
        <a id="raw-markdown-link" class="button" href="bootstrap-node.md">Raw Markdown</a>
      </p>
    </section>
    <section>
      <div id="document-body" class="markdown muted">Loading bootstrap-node.md...</div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="doc-page.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    loadMarkdownDocument("bootstrap-node.md", "Bootstrap Node Guide");
  </script>
</body>
</html>
HTML

cat > "$site_dir/release.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p release process</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="configuration.html">Configuration</a>
        <a href="deployment.html">Deployment</a>
        <a href="benchmarks.html">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1 id="document-title">Release Process</h1>
      <p class="lead muted">Versioning, tagging, source build, and binary release policy.</p>
      <p class="actions">
        <a id="raw-markdown-link" class="button" href="release.md">Raw Markdown</a>
      </p>
    </section>
    <section>
      <div id="document-body" class="markdown muted">Loading release.md...</div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="doc-page.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    loadMarkdownDocument("release.md", "Release Process");
  </script>
</body>
</html>
HTML

cat > "$site_dir/benchmarks.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p benchmarks</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html">Home</a>
        <a href="tester-quickstart.html">Quickstart</a>
        <a href="troubleshooting.html">Troubleshooting</a>
        <a href="connectivity.html">Connectivity</a>
        <a href="configuration.html">Configuration</a>
        <a href="deployment.html">Deployment</a>
        <a href="benchmarks.html" aria-current="page">Benchmarks</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <h1>Benchmarks</h1>
      <p class="lead muted">Latest generated report from the GitHub mirror benchmark workflow.</p>
      <p class="actions">
        <a class="button" href="benchmark-results.md">Markdown report</a>
        <a class="button" href="results.csv">CSV</a>
        <a class="button" href="https://github.com/trevarj/guix-p2p/actions/workflows/benchmarks.yml">Workflow runs</a>
      </p>
    </section>

    <section>
      <h2>Charts</h2>
      <div class="chart-grid">
        <article class="chart-card">
          <h3>Median Elapsed Time</h3>
          <p class="muted">Successful runs grouped by package, HTTP condition, mode, and seed count.</p>
          <div id="elapsed-chart" class="muted">Loading benchmark chart...</div>
        </article>
        <article class="chart-card">
          <h3>Phase Breakdown</h3>
          <p class="muted">Timing phases reported by the VM benchmark harness.</p>
          <div id="phase-chart" class="muted">Loading phase chart...</div>
        </article>
        <article class="chart-card">
          <h3>Payload Throughput</h3>
          <p class="muted">System-build NAR payload bytes divided by import time.</p>
          <div id="throughput-chart" class="muted">Loading throughput chart...</div>
        </article>
        <article class="chart-card">
          <h3>Run History</h3>
          <p class="muted">Median elapsed time from recent successful benchmark artifacts.</p>
          <div id="history-chart" class="muted">Loading history chart...</div>
        </article>
      </div>
    </section>

    <section>
      <h2>Latest Report</h2>
      <div id="report" class="markdown muted">Loading benchmark-results.md...</div>
    </section>

    <section>
      <h2>Recent Runs</h2>
      <p class="muted">Each run keeps its full CSV and markdown output as a GitHub Actions artifact.</p>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Run</th>
              <th>Status</th>
              <th>Started</th>
              <th>Commit</th>
            </tr>
          </thead>
          <tbody id="runs">
            <tr><td colspan="4">Loading workflow runs...</td></tr>
          </tbody>
        </table>
      </div>
    </section>
  </main>

  <script src="markdown.js?v=__SITE_ASSET_VERSION__"></script>
  <script src="charts.js?v=__SITE_ASSET_VERSION__"></script>
  <script>
    async function loadReport() {
      const report = document.getElementById("report");
      const response = await fetch("benchmark-results.md", { cache: "no-store" });
      if (!response.ok) {
        report.textContent = "No benchmark report was published yet.";
        return;
      }
      report.classList.remove("muted");
      report.innerHTML = renderMarkdown(await response.text());
    }

    async function loadRuns() {
      const runsBody = document.getElementById("runs");
      const response = await fetch(
        "https://api.github.com/repos/trevarj/guix-p2p/actions/workflows/benchmarks.yml/runs?per_page=20",
        { cache: "no-store" }
      );
      if (!response.ok) {
        runsBody.innerHTML = "<tr><td colspan=\"4\">Could not load workflow runs.</td></tr>";
        return;
      }
      const payload = await response.json();
      runsBody.innerHTML = payload.workflow_runs.map((run) => {
        const started = new Date(run.created_at).toLocaleString();
        const sha = run.head_sha.slice(0, 7);
        return `<tr>
          <td><a href="${run.html_url}">#${run.run_number}</a></td>
          <td>${run.conclusion || run.status}</td>
          <td>${started}</td>
          <td><a href="${run.head_repository.html_url}/commit/${run.head_sha}">${sha}</a></td>
        </tr>`;
      }).join("");
    }

    loadReport();
    loadBenchmarkCharts();
    loadRuns();
  </script>
</body>
</html>
HTML

for html_file in "$site_dir/configuration.html" "$site_dir/troubleshooting.html" "$site_dir/connectivity.html" "$site_dir/tester-quickstart.html" "$site_dir/deployment.html" "$site_dir/bootstrap-node.html" "$site_dir/release.html" "$site_dir/benchmarks.html"; do
  sed -i "s/__SITE_ASSET_VERSION__/$asset_version/g" "$html_file"
done
