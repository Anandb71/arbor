import openai as oa
from openai import OpenAI

MODEL = "gpt-3.5-turbo"
FALLBACK_MODEL = "gpt-4"

client = OpenAI()
router = oa.OpenAI(base_url="https://openrouter.ai/api/v1", api_key="sk-test-NOT-A-REAL-KEY-0001")

PRICES = {"gpt-3.5-turbo": 0.0005, "gpt-4": 0.03}


def ask(question):
    resp = client.chat.completions.create(model=MODEL, messages=[{"role": "user", "content": question}])
    return resp.choices[0].message.content


def ask_routed(question):
    return router.chat.completions.create(model="openai/gpt-4o", messages=[{"role": "user", "content": question}])


def summarize(text):
    return client.responses.create(model=FALLBACK_MODEL, input=text)


def complete(prompt):
    return client.completions.create(model="gpt-3.5-turbo-instruct", prompt=prompt, max_tokens=64)


def cost(model, tokens):
    return PRICES[model] * tokens
