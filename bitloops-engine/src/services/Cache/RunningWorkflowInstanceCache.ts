import { IRunningWorkflowInstanceCache } from '../interfaces';

/**
 * Simple cache that stores all running Instances
 * without max capacity / removal strategy
 */
class RunningWorkflowInstanceCache implements IRunningWorkflowInstanceCache {
	private _cache: Map<string, boolean>;

	constructor() {
		this._cache = new Map<string, boolean>();
	}
	getCount(): Promise<number> {
		return Promise.resolve(this._cache.size);
	}

	delete(instanceId: string): Promise<void> {
		this._cache.delete(instanceId);
		return Promise.resolve();
	}
	cache(instanceId: string): Promise<void> {
		this._cache.set(instanceId, true);
		return Promise.resolve();
	}
}

export default RunningWorkflowInstanceCache;
