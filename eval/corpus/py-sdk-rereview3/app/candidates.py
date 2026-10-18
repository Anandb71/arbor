"""Values listed with a gap, and whether they are complete (the unit tests
check `values_complete`; here only the gaps are compared)."""
import os

from openai import OpenAI

client = OpenAI()

model = "gpt-4o"
for model in ["gpt-4-0613"]:
    client.chat.completions.create(model=model, messages=[])

TIERS = {"fast-tier": "o1-mini", "default": "gpt-4o"}
client.chat.completions.create(model=TIERS[os.getenv("TIER", "default")], messages=[])

M7 = "gpt-4o"
if os.getenv("Y"):
    M7, temp = "o1-mini", 0
client.chat.completions.create(model=M7, messages=[])

client.chat.completions.create(model="gpt-4o" if os.getenv("F") else "o3-mini", messages=[])

for tier in ("gpt-4o", "o1-mini"):
    client.chat.completions.create(model=tier, messages=[])
