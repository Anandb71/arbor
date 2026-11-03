const { Configuration, OpenAIApi } = require("openai");

const configuration = new Configuration({ apiKey: process.env.OPENAI_API_KEY });
const openai = new OpenAIApi(configuration);

async function complete(prompt) {
  const response = await openai.createCompletion({
    model: "text-davinci-003",
    prompt,
    max_tokens: 64,
  });
  return response.data.choices[0].text;
}

async function chat(messages) {
  return openai.createChatCompletion({ model: "gpt-3.5-turbo", messages });
}

module.exports = { complete, chat };
