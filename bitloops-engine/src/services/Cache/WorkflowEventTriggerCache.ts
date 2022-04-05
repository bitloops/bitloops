import Cache from './Cache';
import { IWorkflowEventTriggerCache } from '../interfaces';
import { EventTriggerWorkflowInfo } from '../../entities/workflow/definitions';

class WorkflowEventTriggerCache extends Cache<EventTriggerWorkflowInfo[]> implements IWorkflowEventTriggerCache {
	private prefixKey = 'workflowEventTrigger';

	constructor(max: number) {
		super(max);
	}

	cache(workspaceId: string, messageId: string, value: EventTriggerWorkflowInfo[]): Promise<void> {
		// const workflowsIds: string[] = this.get(`${workspaceId}:${messageId}`) ?? [];
		// workflowsIds.push(workflowId);
		const key = this.getCacheKey(workspaceId, messageId);
		this.set(key, value);
		return Promise.resolve();
	}
	fetch(workspaceId: string, messageId: string): Promise<EventTriggerWorkflowInfo[]> {
		const key = this.getCacheKey(workspaceId, messageId);
		return Promise.resolve(this.get(key));
	}

	override getSnapshot() {
		console.table(Object.fromEntries(this._cache));
	}

	private getCacheKey(workspaceId: string, messageId: string) {
		return `${this.prefixKey}:${workspaceId}:${messageId}`;
	}
}

export default WorkflowEventTriggerCache;
