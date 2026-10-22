import twilio from "twilio";

const sms = twilio(process.env.TWILIO_SID, process.env.TWILIO_TOKEN);

export function notify(to: string) {
  return sms.messages.create({ body: "Your order shipped", from: "+15005550006", to });
}
