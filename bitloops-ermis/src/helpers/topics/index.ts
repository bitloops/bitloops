import { ERMIS_CONNECTION_PREFIX_TOPIC, WORKFLOW_EVENTS_PREFIX } from "../../constants";

export const getErmisConnectionTopic = (connectionId: string) => {
    return `${ERMIS_CONNECTION_PREFIX_TOPIC}.${connectionId}`;
}

export const getWorkflowEventsTopic = (workspaceId: string, topic: string) => {
    return `${WORKFLOW_EVENTS_PREFIX}.${workspaceId}.${topic}`;
}