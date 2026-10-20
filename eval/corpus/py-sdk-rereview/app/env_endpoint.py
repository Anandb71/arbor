"""The OpenAI SDK reads OPENAI_BASE_URL when the constructor has no base_url."""
import os

from openai import OpenAI

os.environ["OPENAI_BASE_URL"] = "http://localhost:11434/v1"
local = OpenAI()
local.chat.completions.create(model="gpt-4", messages=[])

own = OpenAI(base_url="https://gateway.example/v1")
own.chat.completions.create(model="gpt-4", messages=[])
