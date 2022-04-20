import Bitloops from 'bitloops';

import bitloopsConfig from '../bitloops/bitloopsConfig';
import { DemoChat } from './DemoChat'; 

type TodosCreate = {
  title: string;
  status?: string;
};

type TodosRemove = {
  id: string;
};

type TodosUpdate = {
  title?: string;
  status?: string;
}

type Todo = {
  id: string;
  title: string;
  status: string;
  createdAt: number;
  createdBy: number;
}

const create = (input: TodosCreate): void | Error => {
  return new Error('not implemented');
};

const fetchAll = (): Todo[] | Error => {
  return new Error('not implemented');
};

const update = (input: TodosUpdate): void | Error => {
  return new Error('not implemented');
};

const remove = (input: TodosRemove): void | Error => {
  return new Error('not implemented');
};



const bitloopsApp = Bitloops.initialize(bitloopsConfig);
const demoChat = new DemoChat.DemoChatClient(bitloopsApp);
const bitloops = {
  app: bitloopsApp,
  auth: bitloopsApp.auth,
  subscribe: bitloopsApp.subscribe.bind(bitloopsApp),
  demoChat,
  data: {
    todos: {
        fetchAll,
        create,
        update,
        remove,
    },
  }
};

export { DemoChat };
export default bitloops;
