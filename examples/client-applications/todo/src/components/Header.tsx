import { BitloopsUser } from 'bitloops';
import React from 'react';

interface HeaderProps {
  user: BitloopsUser;
  logout: () => void;
}

function Header(props: HeaderProps) {
  const { user, logout } = props;
  if (user) return (<>
    <div>Hello {user.firstName}</div>
    <button onClick={logout} type="submit">Logout</button>
  </>);
  return null
}

export default Header;
