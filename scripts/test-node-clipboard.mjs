import assert from "node:assert/strict";
import test from "node:test";
import { clipboardNodePayload } from "../src/nodeClipboard.ts";

test("accepts one and multiple v2rayN-compatible share links", () => {
  assert.equal(clipboardNodePayload("vless://id@example.com:443#HK"), "vless://id@example.com:443#HK");
  assert.equal(
    clipboardNodePayload("ss://payload#HK\r\nvmess://payload#US\r\n"),
    "ss://payload#HK\nvmess://payload#US",
  );
  assert.equal(
    clipboardNodePayload("https://user:pass@proxy.example:443#Proxy"),
    "https://user:pass@proxy.example:443#Proxy",
  );
});

test("does not import ordinary URLs or mixed clipboard text", () => {
  assert.equal(clipboardNodePayload("https://github.com/chenzai666/satelite-proxy"), null);
  assert.equal(clipboardNodePayload("https://user:pass@proxy.example/path"), null);
  assert.equal(clipboardNodePayload("https://user:pass@proxy.example:443/path"), "https://user:pass@proxy.example:443/path");
  assert.equal(clipboardNodePayload("vless://id@example.com:443#HK\nhello"), null);
  assert.equal(clipboardNodePayload(""), null);
});

test("accepts v2rayN base64 body for backend parsing", () => {
  assert.equal(clipboardNodePayload("dmxlc3M6Ly9pZEBleGFtcGxlLmNvbQ=="), "dmxlc3M6Ly9pZEBleGFtcGxlLmNvbQ==");
  assert.equal(clipboardNodePayload(Buffer.from("vless://id@example.com:443#HK\nss://payload#US").toString("base64")),
    Buffer.from("vless://id@example.com:443#HK\nss://payload#US").toString("base64"));
  assert.equal(clipboardNodePayload(Buffer.from("ordinary text without a node").toString("base64")), null);
});
