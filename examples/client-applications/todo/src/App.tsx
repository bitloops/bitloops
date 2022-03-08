/* eslint-disable no-await-in-loop */
/* eslint-disable no-restricted-syntax */
/* eslint-disable import/no-named-as-default */
import React, { useState, useEffect } from 'react';
import { v4 as uuid } from 'uuid';
import { Unsubscribe } from 'bitloops/dist/definitions';
import { TodoAppClient } from './bitloops';
import './App.css';
import { Todo } from './bitloops/proto/todoApp';
import bitloopsConfig from './bitloopsConfig';
import TodoPanel from './components/TodoPanel';
import GoogleButton from './components/GoogleButton';
import Header from './components/Header';
import GithubButton from './components/GithubButton';

const ViewStates = {
  ALL: 'All',
  ACTIVE: 'Active',
  COMPLETED: 'Completed',
};

const getBitloopsEventInitialState = (): {
  event: string;
  bitloopsData: any;
} | undefined => undefined;
const getDataInit = (): [] | Todo[] => [];

let publicUnsubscriptions: Unsubscribe[] = [];
let privateUnsubscriptions: Unsubscribe[] = [];

function App() {
  const todoApp = new TodoAppClient(bitloopsConfig);
  const [user, setUser] = useState<any | null>(null);
  const [editable, setEditable] = useState('');
  const [data, setData] = useState(getDataInit());
  const [newValue, setNewValue] = useState('');
  const [bitloopsEvent, setBitloopsEvent] = useState(getBitloopsEventInitialState());

  const loginWithGoogle = () => {
    todoApp.bitloopsApp.auth.authenticateWithGoogle();
  };

  const loginWithGithub = () => {
    todoApp.bitloopsApp.auth.authenticateWithGitHub();
  };

  const clearAuth = () => {
    todoApp.bitloopsApp.auth.clearAuthentication();
  };

  const fetchToDos = async () => {
    const [response, error] = user ? await todoApp.getMine() : await todoApp.getAll();
    if (error) return;
    if (response?.data) setData(response.data);
  };

  async function subscribePublic() {

    const createUnsubscribe = await todoApp.subscribe(todoApp.Events.created(), (d) => setBitloopsEvent({ event: todoApp.Events.created(), bitloopsData: d }));
    const deleteUnsubscribe = await todoApp.subscribe(todoApp.Events.deleted(), (d) => setBitloopsEvent({ event: todoApp.Events.deleted(), bitloopsData: d }));
    const updateUnsubscribe = await todoApp.subscribe(todoApp.Events.updated(), (d) => setBitloopsEvent({ event: todoApp.Events.updated(), bitloopsData: d }));
    publicUnsubscriptions.push(createUnsubscribe, updateUnsubscribe, deleteUnsubscribe);
    console.log('publicUnsubscriptions', publicUnsubscriptions)
    fetchToDos();
  }

  async function subscribeMine() {
    const { uid } = user;
    const myCreatedUnsubscribe = await todoApp.subscribe(todoApp.Events.myCreated(uid), (d) => setBitloopsEvent({ event: todoApp.Events.myCreated(uid), bitloopsData: d }));
    const myDeletedUnsubscribe = await todoApp.subscribe(todoApp.Events.myDeleted(uid), (d) => setBitloopsEvent({ event: todoApp.Events.myDeleted(uid), bitloopsData: d }));
    const myUpdatedUnsubscribe = await todoApp.subscribe(todoApp.Events.myUpdated(uid), (d) => setBitloopsEvent({ event: todoApp.Events.myUpdated(uid), bitloopsData: d }));

    privateUnsubscriptions.push(myCreatedUnsubscribe, myDeletedUnsubscribe, myUpdatedUnsubscribe);
    fetchToDos();
  }

  async function unsubscribePublic() {
    console.log('unsubscribePublic',publicUnsubscriptions.length )
    await Promise.all(publicUnsubscriptions.map(unsubscribeFunc => unsubscribeFunc()));
    publicUnsubscriptions = [];
  }

  async function unsubscribeMine() {
    console.log('unsubscribeMine',privateUnsubscriptions.length)
    await Promise.all(privateUnsubscriptions.map(unsubscribeFunc => unsubscribeFunc()));
    privateUnsubscriptions = [];
  }



  const addItem = async (e: React.MouseEvent<HTMLElement> | React.KeyboardEvent<HTMLInputElement>) => {
    e.preventDefault();
    if (user) {
      await todoApp.createMine({
        status: 'Active',
        text: newValue,
        id: uuid(),
      });
    } else {
      await todoApp.create({
        status: 'Active',
        text: newValue,
        id: uuid(),
      });
    }

    setNewValue('');
  };

  const removeItem = async (id: string) => {
    if (user) {
      await todoApp.deleteMine({ id });
    } else {
      await todoApp.delete({ id });
    }
  };

  const editItem = async (e: any) => {
    const { id } = e.target;
    const { value } = e.target;
    const newData: Todo[] = JSON.parse(JSON.stringify(data));
    for (let i = 0; i < newData.length; i += 1) {
      if (newData[i].id === id) {
        newData[i].text = value;
        if (user) {
          await todoApp.updateMine(newData[i]);
        } else {
          await todoApp.update(newData[i]);
        }
        break;
      }
    }
    setEditable('');
  };

  const updateLocalItem = (e: any) => {
    const { id } = e.target;
    const { value } = e.target;
    const newData: Todo[] = JSON.parse(JSON.stringify(data));
    for (let i = 0; i < newData.length; i += 1) {
      if (newData[i].id === id) {
        newData[i].text = value;
        setData(newData);
        break;
      }
    }
  };

  const handleCheckbox = async (e: any) => {
    const { id } = e.target;
    const { checked } = e.target;
    const newData: Todo[] = JSON.parse(JSON.stringify(data));
    for (let i = 0; i < newData.length; i += 1) {
      if (newData[i].id === id) {
        newData[i].status = checked ? ViewStates.COMPLETED : ViewStates.ACTIVE;
        if (user) {
          await todoApp.updateMine(newData[i]);
        } else {
          await todoApp.update(newData[i]);
        }
      }
    }
  };

  /**
   * Upon initialization set onAuthStateChange in order
   * to keep track of auth state locally
   */
  useEffect(() => {
    // eslint-disable-next-line @typescript-eslint/no-shadow
    todoApp.bitloopsApp.auth.onAuthStateChange((user: any) => {
      setUser(user);
    });
  }, []);

  /**
   * If user exists then unsubscribe public
   * subscriptions and subscribe to mine and
   * vice versa
   */
  useEffect(() => {
    if (user) {
      subscribeMine();
      unsubscribePublic();
    } else {
      subscribePublic();
      unsubscribeMine();
    }
  }, [user]);

  /**
   * Handle each event received appropriately
   */
  useEffect(() => {
    if (bitloopsEvent) {
      const { bitloopsData, event } = bitloopsEvent;
      const updatedArray = JSON.parse(JSON.stringify(data));

      const uid = user?.uid;
      switch (event) {
        case todoApp.Events.created():
        case todoApp.Events.myCreated(uid):
          updatedArray.push(bitloopsData.newData);
          setData(updatedArray);
          break;
        case todoApp.Events.deleted():
        case todoApp.Events.myDeleted(uid):
          for (let i = 0; i < updatedArray.length; i += 1) {
            if (updatedArray[i].id === bitloopsData.id) {
              updatedArray.splice(i, 1);
              break;
            }
          }
          setData(updatedArray);
          break;
        case todoApp.Events.updated():
        case todoApp.Events.myUpdated(uid):
          for (let i = 0; i < updatedArray.length; i += 1) {
            if (updatedArray[i].id === bitloopsData.updatedData.id) {
              updatedArray[i] = bitloopsData.updatedData;
              break;
            }
          }
          setData(updatedArray);
          break;
        default:
          break;
      }
    }
  }, [bitloopsEvent]);

  return (
    <>
      <TodoPanel
        newValue={newValue}
        setNewValue={setNewValue}
        addItem={addItem}
        updateLocalItem={updateLocalItem}
        editItem={editItem}
        removeItem={removeItem}
        editable={editable}
        setEditable={setEditable}
        handleCheckbox={handleCheckbox}
        data={data}
      />
      <Header user={user} logout={clearAuth} />
      {!user && <GoogleButton loginWithGoogle={loginWithGoogle} />}
      {!user && <GithubButton loginWithGithub={loginWithGithub} />}
    </>
  );
}

export default App;
