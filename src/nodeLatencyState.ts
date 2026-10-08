/** 合并本轮测速的不支持标记，不清除其他分组的结果。 */
export function mergeUnsupportedIds(
  previous: ReadonlySet<string>,
  testedIds: Iterable<string>,
  results: Iterable<{ id: string; method?: string | null }>,
): Set<string> {
  const next = new Set(previous);
  for (const id of testedIds) next.delete(id);
  for (const result of results)
    if (result.method === "unsupported") next.add(result.id);
  return next;
}
