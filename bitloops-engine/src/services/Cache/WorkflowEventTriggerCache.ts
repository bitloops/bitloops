import Cache from './Cache';
import { IWorkflowEventTriggerCache } from '../interfaces';
import { EventTriggerWorkflowInfo } from '../../entities/workflow/definitions';

class WorkflowEventTriggerCache extends Cache<EventTriggerWorkflowInfo[]> implements IWorkflowEventTriggerCache {
	constructor(max: number) {
		super(max);
	}

	cache(workspaceId: string, messageId: string, value: EventTriggerWorkflowInfo[]): Promise<void> {
		// const workflowsIds: string[] = this.get(`${workspaceId}:${messageId}`) ?? [];
		// workflowsIds.push(workflowId);
		this.set(`${workspaceId}:${messageId}`, value);
		return Promise.resolve();
	}
	fetch(workspaceId: string, messageId: string): Promise<EventTriggerWorkflowInfo[]> {
		return Promise.resolve(this.get(`${workspaceId}:${messageId}`));
	}

	override getSnapshot() {
		console.table(Object.fromEntries(this._cache));
	}
}

export default WorkflowEventTriggerCache;
