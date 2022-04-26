import { ISubscriptionTopicsCache } from "../../services/Cache/interfaces";
import { Services as TServices } from "../../services/definitions";
import { getErmisConnectionTopic } from "../topics";

export const endConnection = (services: TServices, connectionId: string) => {
    services.sseConnectionsCache.delete(connectionId);
    const topics = services.sseConnectionToTopicsCache.fetch(connectionId);
    if (topics) {
        for (let i = 0; i < topics.length; i++) {
            const topic = topics[i];
            services.sseTopicToConnectionsCache.deleteConnectionIdFromTopic(topic, connectionId);

            const connections = services.sseTopicToConnectionsCache.fetch(topic);
            if (!connections || connections.length === 0) {
                endTopic(services, topic);
            }
        }
    }
    const ermisConnectionTopic = getErmisConnectionTopic(connectionId);
    unsubscribeFromTopic(ermisConnectionTopic, services.subscriptionTopicsCache);

    services.sseConnectionToTopicsCache.delete(connectionId);
};

export const endTopic = (services: TServices, topic: string) => {
    const connections = services.sseTopicToConnectionsCache.fetch(topic);
    if (connections) {
        if (connections.length <= 1) unsubscribeFromTopic(topic, services.subscriptionTopicsCache);
        for (let i = 0; i < connections.length; i++) {
            const connectionId = connections[i];
            services.sseConnectionToTopicsCache.deleteTopicFromConnectionId(connectionId, topic);
        }
    } else {
        unsubscribeFromTopic(topic, services.subscriptionTopicsCache);
    }
    services.sseTopicToConnectionsCache.delete(topic);
}

const unsubscribeFromTopic = (topic: string, subscriptionTopicsCache: ISubscriptionTopicsCache) => {
    const sub = subscriptionTopicsCache.fetch(topic);
    if (sub) sub.unsubscribe();
    subscriptionTopicsCache.delete(topic);
}