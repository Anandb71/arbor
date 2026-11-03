const OpenAI = require("openai");
module.exports = new OpenAI().chat.completions.create({ model: "gpt-3.5-turbo-0301", messages: [] });
