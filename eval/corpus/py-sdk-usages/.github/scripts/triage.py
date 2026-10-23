"""A CI bot under a hidden directory: counted in skipped, not reported."""
from openai import OpenAI

OpenAI().chat.completions.create(model="gpt-4o-mini", messages=[{"role": "user", "content": "triage"}])
