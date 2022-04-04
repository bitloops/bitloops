import { WorkflowDefinition } from '../entities/workflow/WorkflowDefinition';
import { IServices } from '../services/definitions';

export default class Admin {
	private services: IServices;

	constructor(services: IServices) {
		this.services = services;
	}

	callback(data: { command: string; payload: Record<string, any> }): void {
		const { command, payload } = data;
		if (command === 'gc') {
			// console.log('Running gc');
			global.gc();
		} else if (command === 'setOption') {
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
			//TODO add 2 commands
			// 1. upgrade version
			// - update latest mapping (workflowCacheMapping)
			// 2. minor change to already created version 
			// - delete version from cache (workflowCache)
		} else if (command === 'clearWorkflowCache') {
			// console.log(
			// 	'Workflow before delete',
			// 	// TODO update workflowCache to receive environmentId inside key
			// 	this.services.workflowCache.fetch(payload.workflowId, payload.workflowVersion, payload.enviromentId),
			// );
			this.services.workflowCache.delete(payload.workflowId, payload.workflowVersion, payload.environmentId);
			// console.log(
			// 	'Workflow after delete',
			// 	this.services.workflowCache.fetch(payload.workflowId, payload.workflowVersion, payload.enviromentId),
			// );
			console.log('deleted workflow from clear', payload.workflowId);

		} else if (command === 'updateWorkflowCache') {
			// this.services.workflowCache.fetch(payload.workflowId, payload.workflowVersion, payload.enviromentId).then((val) => console.log(val));
			this.services.workflowCache.delete(payload.workflowId, payload.workflowVersion, payload.environmentId);
			console.log('deleted workflow from update', payload.workflowId);
			// this.services.workflowCache.fetch(payload.workflowId, payload.workflowVersion, payload.environmentId).then((val) => console.log(val));
			WorkflowDefinition.get({
				workflowId: payload.workflowId,
				workflowVersion: payload.workflowVersion,
				environmentId: payload.environmentId,
			});
		} else if (command === 'publishToTopic') {
			this.services.mq.publish(payload.replyTo, payload.message);
		}
	}
}
