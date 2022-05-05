import { ISubscriptionTopicsCache } from "../services/Cache/interfaces";
import { IMQ } from "../services/MQ/interfaces";
import { ISubscriptionEvents, SubscribeHandlerType } from "./interfaces";
import { Services as TServices } from "../services/definitions";
import { ERMIS_CONNECTION_TOPIC_ACTIONS } from "../constants";
import { endTopic } from "../helpers/sse";
import { getWorkflowEventsTopic } from "../helpers/topics";

export default class SubscriptionEvents implements ISubscriptionEvents {
    private mq: IMQ;
    private subscriptionTopicsCache: ISubscriptionTopicsCache;

    constructor(mq: IMQ, subscriptionTopicsCache: ISubscriptionTopicsCache) {
        this.mq = mq;
        this.subscriptionTopicsCache = subscriptionTopicsCache;
    }

    public subscribe(topic: string, subscribeHandler: SubscribeHandlerType) {
        if (!this.subscriptionTopicsCache.fetch(topic)) {
            const sub = this.mq.subscribe(topic, subscribeHandler);
            console.log('subscribed to topic', topic);
            this.subscriptionTopicsCache.cache(topic, sub);
        }
    }

    public connectionTopicSubscribeHandler(services: TServices, connectionId: string): SubscribeHandlerType {
        return (data, subject: string): void => {
            const { topic, workspaceId, action } = data;
            const workflowEventsTopic = getWorkflowEventsTopic(workspaceId, topic);

            if (action === ERMIS_CONNECTION_TOPIC_ACTIONS.SUBSCRIBE) {
                services.sseConnectionToTopicsCache.cache(connectionId, workflowEventsTopic);
                services.sseTopicToConnectionsCache.cache(workflowEventsTopic, connectionId);
                this.subscribe(workflowEventsTopic, this.notifySubscribedConnectionsHandler(services, topic, workflowEventsTopic));
            } else if (action === ERMIS_CONNECTION_TOPIC_ACTIONS.UNSUBSCRIBE) {
                endTopic(services, workflowEventsTopic);
            }
        }
    }

    private notifySubscribedConnectionsHandler(services: TServices, topic: string, workflowEventsTopic: string): SubscribeHandlerType {
        return (data, subject: string): void => {
            const connections = services.sseTopicToConnectionsCache.fetch(workflowEventsTopic);
            console.log('topicConnections about to notify', connections);
            for (const connectionId of connections) {
                const { connection } = services.sseConnectionsCache.fetch(connectionId);
                if (!connection) {
                    console.error('Received unexpected connection from cache');
                    continue;
                }
                console.log('topic', topic);
                console.log('data', data);
                if (data.payload) data = data.payload;
                connection.write(`event: ${topic}\n`);
                connection.write(`data: ${JSON.stringify(data)}\n\n`);
            }
        }
    }
}
