from anthropic import Anthropic, AsyncAnthropic


class Assistant:
    MODEL = "claude-3-5-sonnet-20241022"

    def __init__(self, api_key):
        self.client = Anthropic(api_key=api_key)

    def reply(self, prompt):
        message = self.client.messages.create(
            model=self.MODEL,
            max_tokens=512,
            temperature=0.2,
            messages=[{"role": "user", "content": prompt}],
        )
        return message.content[0].text


class AsyncAssistant:
    def __init__(self):
        self.client = AsyncAnthropic()
        self.model = "claude-3-haiku-20240307"

    async def reply(self, prompt):
        return await self.client.messages.create(
            model=self.model, max_tokens=256, messages=[{"role": "user", "content": prompt}]
        )
