import { ERMIS_CONNECTION_PREFIX_TOPIC } from "../constants";
import { ISubscriptionTopicsCache } from "../services/Cache/interfaces";
import { IMQ } from "../services/MQ/interfaces";
import { ISubscriptionEvents, SubscribeHandlerType } from "./interfaces";

export default class SubscriptionEvents implements ISubscriptionEvents {
    private mq: IMQ;
    private subscriptionTopicsCache: ISubscriptionTopicsCache;

    constructor(mq: IMQ, subscriptionTopicsCache: ISubscriptionTopicsCache) {
        this.mq = mq;
        this.subscriptionTopicsCache = subscriptionTopicsCache;
    }

    public subscribe(topic: string, subscribeHandler: SubscribeHandlerType) {
        console.log('subscribed to topic', topic);
        if (!this.subscriptionTopicsCache.fetch(topic)) {
            const sub = this.mq.subscribe(topic, subscribeHandler);
            this.subscriptionTopicsCache.cache(topic, sub);
        }
        // if (!this.subscriptionTopicsCache.fetch(topic)) {
        //     const sub = this.mq.subscribe(topic, (data, subject) => {
        // // TODO implement switch case
        // if (subject === ERMIS_CONNECTION_PREFIX_TOPIC) {
        //     // const topic = `${prefix}.${workspaceId}.${topicName}`;
        //     // if (payload.subscribe) this.subscribe(topic), addToCaches
        //     // if (payload.unsubscribe) this.unsubscribe(topic), deleteCaches
        // }
        // else {
        //     // topic received from engine
        //     // check the connectionId from cache and forward the message through SSE to the client
        // }
        //         console.log('data received', data);
        //         console.log('subject received', subject);
        //     });
        //     this.subscriptionTopicsCache.cache(topic, sub);
        // }
    }

}
