import assert from 'node:assert/strict';
import test from 'node:test';
import { createIpLookupCache } from '../src/ipLookupCache.ts';

test('成功结果按TTL复用，过期后才重取且无持久写入', async () => {
  let time = 0, calls = 0;
  const cache = createIpLookupCache(300, 2, () => time);
  const fetch = async () => ++calls;
  assert.equal(await cache.load('ip', fetch), 1);
  time = 299;
  assert.equal(await cache.load('ip', fetch), 1);
  time = 300;
  assert.equal(await cache.load('ip', fetch), 2);
});
test('合并同路线在途请求，手动刷新绕过成功缓存', async () => {
  const cache = createIpLookupCache(300, 2);
  let resolve, calls = 0;
  const fetch = () => { calls++; return new Promise(r => { resolve = r; }); };
  const first = cache.load('ip', fetch, true, 'routeA');
  const second = cache.load('ip', fetch, true, 'routeA');
  assert.equal(first, second);
  await Promise.resolve();
  assert.equal(calls, 1);
  resolve(1); await first;
  assert.equal(await cache.load('ip', async () => 2, true), 2);
});
test('不同路线不共享请求，失败不缓存，满额淘汰最旧条目', async () => {
  const cache = createIpLookupCache(300, 2);
  let resolve;
  const old = cache.load('ip', () => new Promise(r => { resolve = r; }), true, 'A');
  assert.equal(await cache.load('ip', async () => 2, true, 'B'), 2);
  resolve(1); await old;
  assert.equal(cache.peek('ip'), 2, '旧路线迟到结果不能覆盖新结果缓存');
  await assert.rejects(cache.load('fail', async () => { throw Error('offline'); }));
  assert.equal(cache.peek('fail'), undefined);
  assert.equal(await cache.load('fail', async () => 3), 3);
  await cache.load('new', async () => 4);
  assert.equal(cache.peek('ip'), undefined);
});
test('系统时间倒退使缓存失效', async () => {
  let time = 100;
  const cache = createIpLookupCache(300, 2, () => time);
  await cache.load('ip', async () => 1);
  time = 99; assert.equal(cache.peek('ip'), undefined);
});
test('部分信息使用短TTL，明确类型仍使用正常TTL', async () => {
  let time = 0;
  const cache = createIpLookupCache(v => v === 'unknown' ? 30 : 300, 2, () => time);
  await cache.load('A', async () => 'unknown');
  await cache.load('B', async () => 'business');
  time = 31;
  assert.equal(cache.peek('A'), undefined);
  assert.equal(cache.peek('B'), 'business');
});
