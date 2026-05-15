#!/usr/bin/env bash
set -eu

site_dir="${SITE_DIR:-site}"
report_src="${BENCHMARK_RESULTS_MD:-docs/benchmark-results.md}"
csv_src="${BENCHMARK_RESULTS_CSV:-target/guix-p2p-bench/results.csv}"

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

cp docs/benchmarks.md "$site_dir/benchmark-methodology.md"
cp docs/deployment.md "$site_dir/deployment.md"
cp docs/configuration.md "$site_dir/configuration.md"

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
  gap: 24px;
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
  border-radius: 8px;
  display: block;
  height: 34px;
  padding: 2px 6px;
  width: 136px;
}
nav {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}
nav a {
  border-radius: 999px;
  color: inherit;
  opacity: 0.78;
  padding: 7px 11px;
  text-decoration: none;
}
nav a[aria-current="page"] {
  background: color-mix(in srgb, var(--accent) 20%, transparent);
  opacity: 1;
  font-weight: 650;
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
.hero-mark {
  background: #fbfaf4;
  border: 1px solid var(--border);
  border-radius: 8px;
  display: block;
  height: auto;
  margin-bottom: 18px;
  max-width: min(360px, 82vw);
  padding: 8px 16px;
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
.markdown {
  overflow-wrap: anywhere;
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
  background: var(--surface-muted);
  overflow: auto;
  padding: 16px;
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
  return escapeHtml(value).replace(/`([^`]+)`/g, "<code>$1</code>");
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
  let inFence = false;
  let fence = [];

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

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];

    if (line.startsWith("```")) {
      if (inFence) {
        html.push(`<pre><code>${escapeHtml(fence.join("\n"))}</code></pre>`);
        fence = [];
        inFence = false;
      } else {
        flushParagraph();
        flushList();
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
      continue;
    }

    const tableNext = lines[i + 1] || "";
    if (line.includes("|") && isSeparatorRow(tableNext)) {
      flushParagraph();
      flushList();
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
      const level = heading[1].length;
      html.push(`<h${level}>${renderInline(heading[2])}</h${level}>`);
      continue;
    }

    const item = /^-\s+(.+)$/.exec(line);
    if (item) {
      flushParagraph();
      list.push(item[1]);
      continue;
    }

    paragraph.push(line.trim());
  }

  flushParagraph();
  flushList();
  return html.join("\n");
}
JS

cat > "$site_dir/index.html" <<'HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>guix-p2p documentation</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <header class="site-header">
    <div class="site-header-inner">
      <a class="brand" href="index.html"><img src="assets/guix-p2p-wordmark.svg" alt="guix-p2p"></a>
      <nav aria-label="Site navigation">
        <a href="index.html" aria-current="page">Docs</a>
        <a href="benchmarks.html">Benchmarks</a>
        <a href="https://github.com/trevarj/guix-p2p/actions/workflows/benchmarks.yml">Runs</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <img class="hero-mark" src="assets/guix-p2p-wordmark.svg" alt="guix-p2p">
      <h1>guix-p2p documentation</h1>
      <p class="lead muted">User-facing documentation and benchmark evidence for the GitHub mirror.</p>
      <p class="actions">
        <a class="button" href="benchmarks.html">View latest benchmarks</a>
        <a class="button" href="https://codeberg.org/trevarj/guix-p2p">Source on Codeberg</a>
        <a class="button" href="https://github.com/trevarj/guix-p2p">GitHub mirror</a>
      </p>
    </section>
    <section>
      <h2>Documentation</h2>
      <div class="doc-grid">
        <a class="doc-link" href="benchmarks.html">Benchmarks<span>Rendered latest report, CSV, and recent workflow runs.</span></a>
        <a class="doc-link" href="benchmark-methodology.md">Benchmark methodology<span>How local and VM benchmark suites are run.</span></a>
        <a class="doc-link" href="configuration.md">Configuration<span>Runtime options, paths, and substitute settings.</span></a>
        <a class="doc-link" href="deployment.md">Deployment<span>Bootstrap node and deployment notes.</span></a>
      </div>
    </section>
  </main>
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
        <a href="index.html">Docs</a>
        <a href="benchmarks.html" aria-current="page">Benchmarks</a>
        <a href="https://github.com/trevarj/guix-p2p/actions/workflows/benchmarks.yml">Runs</a>
      </nav>
    </div>
  </header>
  <main>
    <section class="hero">
      <img class="hero-mark" src="assets/guix-p2p-wordmark.svg" alt="guix-p2p">
      <h1>Benchmarks</h1>
      <p class="lead muted">Latest generated report from the GitHub mirror benchmark workflow.</p>
      <p class="actions">
        <a class="button" href="benchmark-results.md">Markdown report</a>
        <a class="button" href="results.csv">CSV</a>
        <a class="button" href="https://github.com/trevarj/guix-p2p/actions/workflows/benchmarks.yml">Workflow runs</a>
      </p>
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

  <script src="markdown.js"></script>
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
    loadRuns();
  </script>
</body>
</html>
HTML
