import anthropic


class ClaudeSampler:
    def __init__(self, model):
        self.model = model
        self.client = anthropic.Anthropic()

    def __call__(self, messages):
        return self.client.messages.create(model=self.model, max_tokens=512, messages=messages)
