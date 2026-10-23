"""A dict of fallback lists expanded with ** at several calls: each call
lists at most 32 nested values and says when more were cut (the **DICT
amplification from the second review of #276)."""
from openai import OpenAI

client = OpenAI()
ROUTED = {
    "extra_body": {
        "fallbacks": [
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
            {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"}, {"model": "gpt-4-0613"},
        ]
    }
}
client.chat.completions.create(model="gpt-4o", **ROUTED, messages=[])
client.chat.completions.create(model="gpt-4o", **ROUTED, messages=[])
client.chat.completions.create(model="gpt-4o", **ROUTED, messages=[])
