from openai import OpenAI

client = OpenAI()


class Sampler:
    def __init__(self, model="gpt-4o-mini", temperature=0.0):
        self.client = OpenAI()
        self.model = model
        self.temperature = temperature

    def __call__(self, messages):
        return self.client.chat.completions.create(
            model=self.model, messages=messages, temperature=self.temperature
        )


def ask(prompt, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[prompt])
