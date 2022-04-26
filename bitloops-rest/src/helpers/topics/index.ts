import { ERMIS_CONNECTION_PREFIX_TOPIC } from '../../constants'
export const getErmisConnectionIdTopic = (connectionId) => {
    return `${ERMIS_CONNECTION_PREFIX_TOPIC}.${connectionId}`
}
