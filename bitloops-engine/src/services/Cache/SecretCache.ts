import { ISecretCache } from '../interfaces';
import Cache from './Cache';

class SecretCache extends Cache<string> implements ISecretCache {
	private prefixKey = 'secret';

	constructor(max: number) {
		super(max);
	}
	fetch(workflowId: string, workflowVersion: string) {
		const key = this.getCacheKey(workflowId, workflowVersion);
		return this.get(key);
	}
	cache(workflowId: string, workflowVersion: string, secrets: any) {
		const key = this.getCacheKey(workflowId, workflowVersion);
		this.set(key, secrets);
	}

	private getCacheKey(workflowId: string, workflowVersion: string) {
		return `${this.prefixKey}:${workflowId}:${workflowVersion}`;
	}
}
export default SecretCache;
