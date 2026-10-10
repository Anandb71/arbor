import { formatName as fmt } from "./format";

export function label(first: string, last: string): string {
  return fmt(first, last);
}
