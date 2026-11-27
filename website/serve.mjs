// Dependency-free, localhost-only preview server. Never use this as a public server.
import http from "node:http";
import { readFile, stat } from "node:fs/promises";
import { dirname, extname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const port = Number(process.env.PORT || 4173);
const types = { ".html": "text/html; charset=utf-8", ".css": "text/css; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".svg": "image/svg+xml", ".png": "image/png" };
const server = http.createServer(async (request, response) => {
  if (request.method !== "GET" && request.method !== "HEAD") {
    response.writeHead(405, { Allow: "GET, HEAD" }).end();
    return;
  }
  try {
    const url = new URL(request.url, "http://localhost");
    const name = decodeURIComponent(url.pathname);
    const target = resolve(root, `.${name === "/" ? "/index.html" : name}`);
    const localPath = relative(root, target);
    if (localPath === ".." || localPath.startsWith(`..${sep}`) || localPath.includes(":")) {
      response.writeHead(403).end("Forbidden");
      return;
    }
    if (!types[extname(target)] || !(await stat(target)).isFile()) throw new Error("Not found");
    const data = await readFile(target);
    response.writeHead(200, { "Content-Type": types[extname(target)], "Content-Length": data.length, "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" });
    response.end(request.method === "HEAD" ? undefined : data);
  } catch {
    response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" }).end("Not found");
  }
});
server.on("error", error => { console.error(error.message); process.exitCode = 1; });
server.listen(port, "127.0.0.1", () => console.log(`SecBack website: http://127.0.0.1:${port}`));
