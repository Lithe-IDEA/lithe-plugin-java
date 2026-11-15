// Dependency-free per-step reports for packaging and the Maven integration lane.
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { performance } from "node:perf_hooks";

const escape = (value) => String(value).replace(/[&<>"']/g, (char) => ({
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&apos;",
})[char]);

export function createReport(lane) {
  const started = performance.now();
  const tests = [];
  const step = async (name, body) => {
    const before = performance.now();
    const test = { name, suite: lane, status: "passed", durationMs: 0 };
    tests.push(test);
    try { return await body(); }
    catch (error) {
      test.status = "failed";
      test.details = error.stack ?? String(error);
      throw error;
    } finally {
      test.durationMs = performance.now() - before;
      console.log(`${test.status === "passed" ? "✓" : "✗"} ${name} (${test.durationMs.toFixed(1)} ms)`);
    }
  };
  const write = () => {
    const directory = resolve(process.env.LITHE_REPORT_DIR ?? ".artifacts/test-stability");
    mkdirSync(directory, { recursive: true });
    const durationMs = performance.now() - started;
    const failures = tests.filter((test) => test.status === "failed").length;
    const errors = tests.filter((test) => test.status === "error").length;
    const report = { schemaVersion: 1, runner: "node", lane, warnMs: 1_000, maxMs: 610_000, durationMs, tests };
    writeFileSync(join(directory, `${lane}.json`), `${JSON.stringify(report, null, 2)}\n`);
    const cases = tests.map((test) => {
      const tag = test.status === "error" ? "error" : "failure";
      return `  <testcase classname="${escape(test.suite)}" name="${escape(test.name)}" time="${test.durationMs / 1000}">${test.details ? `<${tag} message="step failed">${escape(test.details)}</${tag}>` : ""}</testcase>`;
    });
    writeFileSync(join(directory, `${lane}.junit.xml`), `<?xml version="1.0" encoding="UTF-8"?>\n<testsuite name="${escape(lane)}" tests="${tests.length}" failures="${failures}" errors="${errors}" time="${durationMs / 1000}">\n${cases.join("\n")}\n</testsuite>\n`);
    const rows = tests.map((test) => `<tr><td>${escape(test.name)}</td><td>${test.durationMs.toFixed(1)}</td><td>${test.status}${test.details ? `<pre>${escape(test.details)}</pre>` : ""}</td></tr>`);
    writeFileSync(join(directory, `${lane}.html`), `<!doctype html><html lang="en"><meta charset="utf-8"><title>${escape(lane)}</title><style>body{font:16px system-ui;margin:2rem}table{border-collapse:collapse;width:100%}td,th{text-align:left;padding:.5rem;border-bottom:1px solid #aaa}pre{white-space:pre-wrap}</style><h1>${escape(lane)}</h1><p>${tests.length} steps, ${failures} failures, ${errors} errors, ${durationMs.toFixed(1)} ms total</p><table><thead><tr><th>Step</th><th>Duration (ms)</th><th>Result</th></tr></thead><tbody>${rows.join("\n")}</tbody></table></html>\n`);
    console.log(`Timing reports: ${join(directory, `${lane}.html`)} (JSON, JUnit alongside)`);
  };
  const failure = (name, error) => {
    tests.push({ name, suite: lane, status: "error", durationMs: 0, details: error.stack ?? String(error) });
  };
  return { step, write, failure };
}
