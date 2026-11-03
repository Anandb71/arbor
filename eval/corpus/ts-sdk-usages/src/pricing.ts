export const PRICES: Record<string, number> = {
  "gpt-4o": 0.0025,
  "gpt-4o-mini": 0.00015,
  "claude-2.1": 0.008,
};

export function price(model: string, tokens: number) {
  return (PRICES[model] ?? 0) * tokens;
}
