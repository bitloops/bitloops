pub fn  badly_formatted (  x:i32,y : i32)->i32{if x>y {x+y}else{ x-y }}

pub struct  Demo { pub value:i32 }

impl Demo{pub fn new( value:i32)->Self{Self{value}}
    pub fn  bump(&mut self){self.value = self.value+1;}
}
