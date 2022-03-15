import React from 'react';

interface HeaderProps {
  user: any;
  logout: () => void;
}

export const Header: React.FC<HeaderProps> = (props): JSX.Element => {
  const { user, logout } = props;
  if (user) return (<div style={{display: 'flex'}}>
    <div style={{padding: 20}}>Hello {user.firstName}</div>
    <button style={{marginTop: 20, marginBottom: 20}} onClick={logout}>Logout</button>
  </div>);
  else return (<></>);
};

export default Header;
