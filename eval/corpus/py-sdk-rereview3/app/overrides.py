"""A wildcard import may rebind any module-level name written before it."""
MODEL4 = "gpt-4o"
from local_overrides import *  # noqa: E402,F403

from openai import OpenAI  # noqa: E402

client = OpenAI()
client.chat.completions.create(model=MODEL4, messages=[])
MODEL9 = "gpt-4.1"
client.chat.completions.create(model=MODEL9, messages=[])
