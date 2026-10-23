import anthropic

client = anthropic.Anthropic()
client.messages.create(model="claude-2.1", max_tokens=10, messages=[])
