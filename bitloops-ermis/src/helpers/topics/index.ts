import { ERMIS_CONNECTION_PREFIX_TOPIC } from "../../constants";

export const getErmisConnectionTopic = (connectionId: string) => {
    return `${ERMIS_CONNECTION_PREFIX_TOPIC}.${connectionId}`;
}