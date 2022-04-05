import { WorkflowDefinition } from '../entities/workflow/WorkflowDefinition';
import { IServices } from '../services/definitions';
import { ADMIN_COMMANDS } from '../constants';

export default class Admin {
	private services: IServices;

	constructor(services: IServices) {
		this.services = services;
	}

	callback(data: { command: string; payload: Record<string, any> }): void {
		const { command, payload } = data;

		switch (command) {
			case ADMIN_COMMANDS.GC: {
				global.gc();
				break;
			}
			case ADMIN_COMMANDS.SET_OPTION: {
				try {
					if (
						payload.key &&
						payload.value &&
						(!payload.serverUUID || payload.serverUUID === this.services.Options.getServerUUID())
					) {
						this.services.Options.setOption(payload.key, payload.value);
					}
				} catch (error) {
					console.error('Could not parse payload for setOption', error);
				}
				break;
			}
			case ADMIN_COMMANDS.CLEAR_WORKFLOW_CACHE: {
				this.services.workflowCache.delete(payload.workflowId, payload.workflowVersion);
				console.log('deleted workflow from clear', payload.workflowId);
				break;
			}
			case ADMIN_COMMANDS.UPDATE_WORFKLOW_CACHE: {
				this.services.workflowCache.fetch(payload.workflowId, payload.workflowVersion, payload.enviromentId).then((val) => console.log(val));
				this.services.workflowCache.delete(payload.workflowId, payload.workflowVersion);
				console.log('deleted workflow from update', payload.workflowId);
				// this.services.workflowCache.fetch(payload.workflowId, payload.workflowVersion, payload.environmentId).then((val) => console.log(val));
				WorkflowDefinition.get({
					workflowId: payload.workflowId,
					workflowVersion: payload.workflowVersion,
					environmentId: payload.environmentId,
				});
				break;
			}
			case ADMIN_COMMANDS.UPDATE_WORFKLOW_VERSION_MAPPING_CACHE: {
				this.services.workflowVersionMappingCache.delete(payload.workflowId);
				this.services.workflowVersionMappingCache.cache(payload.workflowId, payload.workflowVersion);
				console.log(`cached workflow version mapping with id: ${payload.workflowId} and version: ${payload.workflowVersion}`);
				break;
			}
			case ADMIN_COMMANDS.PUBLISH_TO_TOPIC: {
				this.services.mq.publish(payload.replyTo, payload.message);
				break;
			}
			default:
				console.error(`Admin command ${command} - not implemented`);
				break;
		}
	}
}
