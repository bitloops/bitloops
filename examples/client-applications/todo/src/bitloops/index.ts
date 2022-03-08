import Bitloops from 'bitloops';

import type {
  CreateMineRequest, CreateMineResponse, DeleteMineRequest,
  DeleteMineResponse, GetMineResponse, UpdateMineRequest,
  UpdateMineResponse, UpdateResponse, UpdateRequest,
  GetAllResponse, GetAllRequest, DeleteResponse,
  DeleteRequest, CreateResponse, CreateRequest,
} from './proto/todoApp';

export interface ITodoAppClient {
  /**
   * @generated from Bitloops Protobuf: Create(CreateRequest) returns (CreateResponse);
   */
  create(input: CreateRequest): Promise<[response: CreateResponse | null, error: any | null]>;
  /**
   * @generated from Bitloops Protobuf: Delete(DeleteRequest) returns (DeleteResponse);
   */
  delete(input: DeleteRequest): Promise<[response: DeleteResponse | null, error: any | null]>;
  /**
   * @generated from Bitloops Protobuf: GetAll(GetAllRequest) returns (GetAllResponse);
   */
  getAll(input: GetAllRequest): Promise<[response: GetAllResponse | null, error: any | null]>;
  /**
   * @generated from Bitloops Protobuf: Update(UpdateRequest) returns (UpdateResponse);
   */
  update(input: UpdateRequest): Promise<[response: UpdateResponse | null, error: any | null]>;
}

export class TodoAppClient implements ITodoAppClient {
  bitloopsApp: Bitloops;

  Events: {
    created: () => string,
    deleted: () => string,
    updated: () => string,
    /* eslint-disable */
    myCreated: (uid:string) => string,
    myUpdated: (uid:string) => string,
    myDeleted: (uid:string) => string
  };

  constructor(bitloopsConfig: any) {
    this.bitloopsApp = Bitloops.initialize(bitloopsConfig);
    this.Events = {
      created: () => 'workflow-events.ToDos.created',
      deleted: () => 'workflow-events.ToDos.deleted',
      updated: () => 'workflow-events.ToDos.updated',
      myCreated: (uid: string) => `workflow-events.ToDos.created.${uid}`,
      myDeleted: (uid: string) => `workflow-events.ToDos.deleted.${uid}`,
      myUpdated: (uid: string) => `workflow-events.ToDos.updated.${uid}`,
    };
  }

  async subscribe(namedEvent: string, callback: (data: any) => void): Promise<any> {
    return this.bitloopsApp.subscribe(namedEvent, callback);
  }

  /**
   * @generated from Bitloops Protobuf: Create(CreateRequest) returns (CreateResponse);
   */
  async create(input: CreateRequest):
    Promise<[response: CreateResponse | null, error: any | null]> {
    try {
      const response: CreateResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        '9ea7d8d2-62db-4210-809f-6fc79b173c6d',
        input,
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }

  /**
   * @generated from Bitloops Protobuf: Delete(DeleteRequest) returns (DeleteResponse);
   */
  async delete(input: DeleteRequest):
    Promise<[response: DeleteResponse | null, error: any | null]> {
    try {
      const response: DeleteResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        'f583c300-b21e-4a3b-ac7d-56f5489f530c',
        input,
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }

  /**
   * @generated from Bitloops Protobuf: GetAll(GetAllRequest) returns (GetAllResponse);
   */
  async getAll(): Promise<[response: GetAllResponse | null, error: any | null]> {
    try {
      const response: GetAllResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        '616bb291-9521-4724-985c-b2048dde56a8',
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }

  /**
   * @generated from Bitloops Protobuf: Update(UpdateRequest) returns (UpdateResponse);
   */
  async update(input: UpdateRequest):
    Promise<[response: UpdateResponse | null, error: any | null]> {
    try {
      const response: UpdateResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        'f9a51c7c-6b12-408f-a5ee-fb164cbabcc6',
        input,
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }

  /**
   * @generated from Bitloops Protobuf: Create(CreateMineRequest) returns (CreateMineResponse);
   */
  async createMine(input: CreateMineRequest):
    Promise<[response: CreateMineResponse | null, error: any | null]> {
    try {
      const response: CreateMineResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        '1761f542-9c8e-41ea-8ce1-b2dd3a93d797',
        input,
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }

  /**
     * @generated from Bitloops Protobuf: Delete(DeleteMineRequest) returns (DeleteMineResponse);
     */
  async deleteMine(input: DeleteMineRequest):
    Promise<[response: DeleteMineResponse | null, error: any | null]> {
    try {
      const response: DeleteMineResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        '5a479114-4624-4455-83de-ab843a19567b',
        input,
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }

  /**
     * @generated from Bitloops Protobuf: GetAll(GetMineRequest) returns (GetMineResponse);
     */
  async getMine(): Promise<[response: GetMineResponse | null, error: any | null]> {
    try {
      const response: GetMineResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        'c8cf52b6-6b74-44e9-a735-01eba8d2cf8e',
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }

  /**
     * @generated from Bitloops Protobuf: Update(UpdateRequest) returns (UpdateResponse);
     */
  async updateMine(input: UpdateMineRequest):
    Promise<[response: UpdateMineResponse | null, error: any | null]> {
    try {
      const response: UpdateMineResponse = await this.bitloopsApp.request(
        '88e761bf-4824-4974-96b4-45c7bf741f11',
        '03130b74-77ee-4489-9038-65f922082afc',
        input,
      );
      return [response, null];
    } catch (error) {
      console.error(error);
      return [null, error];
    }
  }
}
