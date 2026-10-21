import OpenAI from "openai";

jest.mock("openai");

const client = new OpenAI();

describe("assistant", () => {
  it("asks the model", async () => {
    await client.chat.completions.create({ model: "gpt-4o", messages: [] });
  });
});
