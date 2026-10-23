import { formatName } from "./format";

export function label(first: string, last: string): string {
  return formatName(first, last);
}
