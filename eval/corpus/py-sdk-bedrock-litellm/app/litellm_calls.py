"""LiteLLM calls (synthetic)."""
import litellm
from litellm import completion

FALLBACK = "anthropic/claude-3-5-haiku-20241022"

litellm.drop_params = True


def ask(question):
    return completion(model="gpt-4o-mini", messages=[{"role": "user", "content": question}])


async def ask_async(question):
    return await litellm.acompletion(FALLBACK, messages=[{"role": "user", "content": question}])


def count(text):
    return litellm.token_counter(model="gpt-4o", text=text)
