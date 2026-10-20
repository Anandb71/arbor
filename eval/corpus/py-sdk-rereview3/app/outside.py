"""Writes from outside a class: `bot.model = ...`, `setattr` with a run-time
name, and `setattr(client, "base_url", ...)`."""
from openai import OpenAI

client = OpenAI()


class Bot:
    model = "gpt-4o"

    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])


bot = Bot()
bot.model = "o1-mini"
bot.go()


class Tuned:
    engine = "gpt-4o"

    def __init__(self, **options):
        for name, value in options.items():
            setattr(self, name, value)

    def go(self):
        return client.chat.completions.create(engine=self.engine, messages=[])


router = OpenAI()
setattr(router, "base_url", "https://gateway.example/v1")
router.chat.completions.create(model="gpt-4", messages=[])
