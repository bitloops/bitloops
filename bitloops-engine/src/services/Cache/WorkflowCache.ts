import { IBitloopsWorkflowDefinition } from '../../entities/workflow/definitions';
import { IWorkflowCache } from '../interfaces';
import Cache from './Cache';

class WorkflowCache extends Cache<Record<string, IBitloopsWorkflowDefinition>> implements IWorkflowCache {
	private prefixKey = 'workflow';

	constructor(max: number) {
		super(max);
	}

	async cache(workflowId: string, workflowVersion: string, environmentId: string, workflow: IBitloopsWorkflowDefinition): Promise<void> {
		console.log(`adding worfklow with id: ${workflowId} to cache version ${workflowVersion} and environment ${environmentId}`);
		let workflowPerEnv = await this.fetchWorkflowPerEnv(workflowId, workflowVersion);
		if (!workflowPerEnv) workflowPerEnv = {};
		workflowPerEnv[environmentId] = workflow;
		const key = this.getCacheKey(workflowId, workflowVersion);
		this.set(key, workflowPerEnv);
	}

	private fetchWorkflowPerEnv(workflowId: string, workflowVersion: string): Promise<Record<string, IBitloopsWorkflowDefinition>> {
		const key = this.getCacheKey(workflowId, workflowVersion);
		const res = this.get(key);
		return Promise.resolve(res);
	}

	fetch(workflowId: string, workflowVersion: string, environmentId: string): Promise<IBitloopsWorkflowDefinition | null> {
		console.log(`fetching worfklow with id: ${workflowId},  version ${workflowVersion} and environment ${environmentId}`);
		const key = this.getCacheKey(workflowId, workflowVersion);
		const res = this.get(key);
		if (!res) return Promise.resolve(null);
		return Promise.resolve(res[environmentId]);
	}

	delete(workflowId: string, workflowVersion: string) {
		console.log(`deleting worfklow with id: ${workflowId} from cache version ${workflowVersion}`);
		const key = this.getCacheKey(workflowId, workflowVersion);
		this.remove(key);
	}

	private getCacheKey(workflowId: string, workflowVersion: string) {
		return `${this.prefixKey}:${workflowId}:${workflowVersion}`;
	}
}

export default WorkflowCache;
