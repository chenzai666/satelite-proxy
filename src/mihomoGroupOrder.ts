/** Move one displayed group; unrelated and filtered-out groups retain order. */
export function moveMihomoGroup(order: string[], id: string, target: string, after: boolean): string[] {
  if (id === target || !order.includes(id) || !order.includes(target)) return order;
  const next = order.filter(name => name !== id);
  next.splice(next.indexOf(target) + (after ? 1 : 0), 0, id);
  return next;
}
