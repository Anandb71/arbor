import Anthropic from "@anthropic-ai/sdk";

export class Assistant {
  private client = new Anthropic();
  protected model = "claude-3-5-sonnet-20241022";

  async reply(prompt: string) {
    const message = await this.client.messages.create({
      model: this.model,
      max_tokens: 1024,
      messages: [{ role: "user", content: prompt }],
    });
    return message.content;
  }
}

class Summarizer {
  private readonly anthropic: Anthropic;
  private readonly model = "claude-3-haiku-20240307";

  constructor() {
    this.anthropic = new Anthropic({ maxRetries: 3 });
  }

  summarize(text: string) {
    return this.anthropic.messages.stream({
      model: this.model,
      max_tokens: 512,
      messages: [{ role: "user", content: text }],
    });
  }
}

export const summarizer = () => new Summarizer();
