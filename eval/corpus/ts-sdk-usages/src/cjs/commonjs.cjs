const OpenAI = require("openai");
const { AzureOpenAI } = require("openai");

const client = new OpenAI();
const azure = new AzureOpenAI({ apiVersion: "2024-10-21" });

exports.ask = async function ask(question) {
  const response = await client.chat.completions.create({
    model: "gpt-4",
    messages: [{ role: "user", content: question }],
  });
  await azure.chat.completions.create({ model: "gpt-35-turbo", messages: [] });
  return response;
};
