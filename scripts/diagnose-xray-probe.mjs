// CI-only synthetic configuration; no user nodes, credentials or global proxy.
import net from "node:net";
import http from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import assert from "node:assert/strict";
const bin = process.argv[2];
const upstream = net.createServer(socket => {
  let data = Buffer.alloc(0), step = 0;
  socket.on("data", chunk => {
    data = Buffer.concat([data, chunk]);
    if (step === 0 && data.length >= 2 + data[1]) {
      data = data.subarray(2 + data[1]); socket.write(Buffer.from([5, 0])); step = 1;
    }
    if (step === 1 && data.length >= 5) {
      const len = data[3] === 1 ? 10 : data[3] === 4 ? 22 : 7 + data[4];
      if (data.length < len) return;
      data = data.subarray(len); socket.write(Buffer.from([5, 0, 0, 1, 127, 0, 0, 1, 0, 80])); step = 2;
    }
    if (step === 2 && data.includes("\r\n\r\n")) {
      console.log("Synthetic upstream received HTTP");
      socket.end("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); step = 3;
    }
  });
  socket.on("error", () => {});
});
upstream.listen(0, "127.0.0.1"); await once(upstream, "listening");
const reserve = net.createServer(); reserve.listen(0, "127.0.0.1"); await once(reserve, "listening");
const port = reserve.address().port;
const config = JSON.stringify({
  log: { loglevel: "debug" },
  inbounds: [{ tag: "probe", listen: "127.0.0.1", port, protocol: "http", settings: { accounts: [{ user: "satelite-probe", pass: "fixture-only" }] } }],
  outbounds: [{ tag: "node-fixture", protocol: "socks", settings: { servers: [{ address: "127.0.0.1", port: upstream.address().port }] } }],
  routing: { domainStrategy: "AsIs", rules: [{ type: "field", inboundTag: ["probe"], outboundTag: "node-fixture" }] },
});
const validate = spawnSync(bin, ["run", "-test", "-c", "stdin:"], { input: config, encoding: "utf8", timeout: 10000, windowsHide: true });
console.log("Config validation", validate.status, validate.stdout, validate.stderr);
assert.equal(validate.status, 0);
await new Promise(resolve => reserve.close(resolve));
const child = spawn(bin, ["run", "-c", "stdin:"], { windowsHide: true });
child.stdout.on("data", chunk => process.stdout.write(chunk));
child.stderr.on("data", chunk => process.stderr.write(chunk));
child.stdin.end(config);
try {
  for (let i = 0; i < 60; i++) {
    const ready = await new Promise(resolve => { const s = net.connect(port, "127.0.0.1"); s.on("connect", () => { s.destroy(); resolve(true); }); s.on("error", () => resolve(false)); });
    if (ready) break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  const status = await new Promise((resolve, reject) => {
    const req = http.get({ hostname: "127.0.0.1", port, path: "http://probe.invalid/generate_204", headers: { Host: "probe.invalid", "Proxy-Authorization": `Basic ${Buffer.from("satelite-probe:fixture-only").toString("base64")}` } }, res => { res.resume(); resolve(res.statusCode); });
    req.setTimeout(3000, () => req.destroy(new Error("probe timeout"))); req.on("error", reject);
  });
  console.log("Proxy status", status); assert.equal(status, 204);
} finally {
  child.kill(); await once(child, "close"); upstream.close();
}
