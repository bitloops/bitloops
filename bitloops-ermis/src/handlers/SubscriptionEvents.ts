import { IMQ } from "../services/MQ/interfaces";

export default class SubscriptionEvents {
    private mq: IMQ;

    constructor(mq: IMQ) {
        this.mq = mq;
    }

    public subscribe(topic: string) {
        console.log('subscribed to topic', topic);
        this.mq.subscribe(topic, (data, subject) => {
            console.log('data received', data);
            console.log('subject received', subject);
        });
    }
}
