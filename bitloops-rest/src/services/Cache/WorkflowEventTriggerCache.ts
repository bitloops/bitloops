import Cache from './LRUCache';
import { IWorkflowEventTriggerCache } from '../interfaces';

class WorkflowEventTriggerCache extends Cache<string[]> implements IWorkflowEventTriggerCache {
	constructor(max: number) {
		super(max);
	}

	cache(workspaceId: string, messageId: string, workflowId: string): Promise<void> {
		const workflowsIds: string[] = this.get(`${workspaceId}:${messageId}`) ?? [];
		workflowsIds.push(workflowId);
		this.set(`${workspaceId}:${messageId}`, workflowsIds);
		return Promise.resolve();
	}
	fetch(workspaceId: string, messageId: string): Promise<string[]> {
		return Promise.resolve(this.get(`${workspaceId}:${messageId}`));
	}
}

export default WorkflowEventTriggerCache;