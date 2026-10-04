const units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];

export function formatBytes(n: number): string {
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  const digits = i >= 3 && n < 10 ? 1 : 0;
  return `${n.toFixed(digits).replace(".", ",")} ${units[i]}`;
}

/** Russian plural: plural(5, "мод", "мода", "модов"). */
export function plural(n: number, one: string, few: string, many: string): string {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}
