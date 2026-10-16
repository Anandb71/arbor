from twilio.rest import Client

twilio = Client("ACXXXXXXXX", "not-a-real-token")


def send(body):
    return twilio.messages.create(body=body, from_="+15550000000", to="+15551111111")
