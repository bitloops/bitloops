// import WorkflowEventTriggerCache from '../WorkflowEventTriggerCache';
// import { v4 as uuidv4 } from 'uuid';

// const WORKSPACE_1 = 'workspaceId1';
// const MESSAGE_1 = 'message1';
// const MESSAGE_2 = 'message2';
// const MESSAGE_3 = 'message3';

// const WORKFLOW_1 = [{ workflowId: '1' }];
// const WORKFLOW_2 = [{ workflowId: '2' }];
// describe('WorkflowEventTriggerCache', () => {
// 	let cache: WorkflowEventTriggerCache;
// 	beforeAll(() => {
// 		cache = new WorkflowEventTriggerCache(3);
// 	});

// 	describe('caching new values', () => {
// 		it('should cache 3 mapping entities', () => {
// 			cache.cache(WORKSPACE_1, MESSAGE_1, WORKFLOW_1);
// 			let cnt: number = cache.getCount();
// 			expect(cnt).toBe(1);
// 			cache.cache(WORKSPACE_1, MESSAGE_1, WORKFLOW_2);
// 			cnt = cache.getCount();
// 			expect(cnt).toBe(1);
// 			cache.cache(WORKSPACE_1, MESSAGE_2, WORKFLOW_1);
// 			cache.cache(WORKSPACE_1, MESSAGE_3, WORKFLOW_1);
// 			cnt = cache.getCount();
// 			expect(cnt).toBe(3);
// 		});
// 	});

// 	describe('fetching values', () => {
// 		it('should read value and update oldest properly', async () => {
// 			const res = await cache.fetch(WORKSPACE_1, MESSAGE_1);
// 			expect([WORKFLOW_1, WORKFLOW_2]).toEqual(res);
// 		});

// 		it("should return null if value isn't cached", async () => {
// 			const res = await cache.fetch(WORKSPACE_1, MESSAGE_1 + 'adsff');
// 			expect(res).toBeNull();
// 		});
// 	});
// 	/**
// 	 * NEW LRU
// 	 * WORKSPACE_1 , MESSAGE_2
// 	 */

// 	describe('caching new value and pop older', () => {
// 		it('should remove oldest item', async () => {
// 			cache.cache(WORKSPACE_1, 'newMessage', WORKFLOW_1);
// 			let cnt: number = cache.getCount();
// 			expect(cnt).toBe(3);
// 			/**
// 			 * NEW LRU
// 			 * WORKSPACE_1, MESSAGE_3
// 			 */

// 			const res = await cache.fetch(WORKSPACE_1, MESSAGE_2);
// 			expect(res).toBeNull();
// 		});
// 	});

// 	describe('deleting values', () => {
// 		it('should delete value and adjust cache size', async () => {
// 			cache.remove(`${WORKSPACE_1}:${MESSAGE_3}`);
// 			expect(cache.getCount()).toBe(2);
// 		});

// 		it('should do nothing when deleting an unknown key', async () => {
// 			const initialSize = cache.getCount();
// 			cache.remove(uuidv4());
// 			expect(cache.getCount()).toBe(initialSize);
// 		});
// 	});

// 	describe('resizing cache', () => {
// 		it('should not be able to read because no getter is defined', () => {
// 			expect(cache.max).toBeUndefined();
// 		});
// 		it('should drop LRU item when setting size smaller map.size', async () => {
// 			cache.max = 1;
// 			expect(cache.getCount()).toBe(1);
// 			const res = await cache.fetch(WORKSPACE_1, 'newMessage');
// 			expect(res).toEqual([WORKFLOW_1]);
// 		});
// 	});

// 	describe('clearing cache', () => {
// 		it('should delete all keys', () => {
// 			cache.clear();
// 			expect(cache.getCount()).toBe(0);
// 		});
// 	});
// });
