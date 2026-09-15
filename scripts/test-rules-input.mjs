import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";

const compiled = ts.transpileModule(await readFile("src/rulePayload.ts", "utf8"), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { parsePayloadEntries: parse, normalizeIpCidrEntry: cidr, validatePayloadEntries: validate } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`);

test("批量规则分隔、进程名及关键词中的空格", () => {
  assert.deepEqual(parse("a.com b.com\n c.com ", "domain"), ["a.com", "b.com", "c.com"]);
  assert.deepEqual(parse("My App.exe\nOther.exe", "process"), ["My App.exe", "Other.exe"]);
  assert.deepEqual(parse("my keyword\nother", "domain_keyword"), ["my keyword", "other"]);
  assert.deepEqual(parse("  \n ", "domain"), []);
});
test("地址族及 CIDR 上下界校验", () => {
  assert.equal(cidr("1.2.3.4"), "1.2.3.4/32");
  assert.equal(cidr("2001:db8::1"), "2001:db8::1/128");
  for (const value of ["1.2.3.4/33", "2001:db8::1/129", "256.1.1.1", "01.2.3.4", "1.2.3.4/-1", "1.2.3.4/24/1", "garbage"]) assert.equal(cidr(value), null, value);
  assert.equal(cidr("0.0.0.0/0"), "0.0.0.0/0");
  assert.equal(cidr("::/0"), "::/0");
});
test("域名形状、标签长度和进程路径校验", () => {
  assert.deepEqual(validate(["github.com", "例子.测试", "localhost"], "domain"), []);
  assert.equal(validate(["https://a.com", ".com", "a..com", "-a.com", `${"a".repeat(64)}.com`], "domain_suffix").length, 5);
  assert.deepEqual(validate(["My App.exe"], "process"), []);
  assert.equal(validate(["C:\\My App.exe"], "process").length, 1);
});
