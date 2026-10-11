//! About-only Sixel artwork, using the precomputed indexed export of the GUI logo.
use ratatui::layout::Rect;
use crossterm::terminal::WindowSize;
use std::fmt::Write;

pub fn supported(env:impl Fn(&str)->Option<String>,launcher:impl FnOnce()->bool)->bool {
    match env("LENZ_IMAGES").as_deref(){Some("sixel")=>return true,Some("cells"|"kitty")=>return false,_=>{}}
    if env("TMUX").is_some()||env("STY").is_some(){return false;}
    env("TERM").is_some_and(|term|term=="foot"||term.starts_with("foot-"))||launcher()
}
pub fn foot_launcher()->bool {
    // Crossterm consumes DA replies without exposing capabilities; don't race its input owner.
    #[cfg(target_os="linux")]
    {
        let mut pid=std::process::id();
        for _ in 0..16 {
            let Ok(stat)=std::fs::read_to_string(format!("/proc/{pid}/stat"))else{break;};
            let Some((_,tail))=stat.rsplit_once(") ")else{break;};
            if stat.split_once('(').is_some_and(|(_,name)|name.starts_with("foot) ")||name.starts_with("footclient) ")){return true;}
            let Some(parent)=tail.split_whitespace().nth(1).and_then(|p|p.parse::<u32>().ok())else{break;};
            if parent<=1||parent==pid{break;}pid=parent;
        }
    }
    false
}

pub struct Logo { pub rect:Rect, pub size:(u16,u16,u16,u16), pub background:u32, pub sequence:String }
impl Logo {
    pub fn matches(&self,rect:Rect,size:&WindowSize,background:u32)->bool {
        self.rect==rect&&self.size==(size.columns,size.rows,size.width,size.height)&&self.background==background
    }
    pub fn new(rect:Rect,size:&WindowSize,background:u32)->Option<Self> {
        if rect.is_empty()||size.columns==0||size.rows==0||size.width==0||size.height==0{return None;}
        let cw=usize::from(size.width)/usize::from(size.columns);
        let ch=usize::from(size.height)/usize::from(size.rows);
        let side=(usize::from(rect.width)*cw).min(usize::from(rect.height)*ch).min(384);
        if side==0{return None;}
        let asset=include_bytes!("../../../assets/lenz-about-sixel.bin");
        // Sixel has no alpha channel: composite the exported palette onto the dialog background.
        let mut sequence=format!("\x1b7\x1b[{};{}H\x1bP0;1;0q\"1;1;{side};{side}",rect.y+1,rect.x+1);
        for (index,pixel) in asset[..1024].as_chunks::<4>().0.iter().enumerate(){
            let alpha=u32::from(pixel[3]);
            let rgb=[16,8,0].map(|shift|{let channel=match shift{16=>pixel[0],8=>pixel[1],_=>pixel[2]};let value=(u32::from(channel)*alpha+((background>>shift)&255)*(255-alpha)+127)/255;(value*100+127)/255});
            let _=write!(sequence,"#{index};2;{};{};{}",rgb[0],rgb[1],rgb[2]);
        }
        let mut planes=vec![0u8;256*side];
        for y in (0..side).step_by(6) {
            planes.fill(0);let mut used=[false;256];
            for dy in 0..6.min(side-y){for x in 0..side {
                let index=usize::from(asset[1024+((y+dy)*96/side)*96+x*96/side]);
                planes[index*side+x]|=1<<dy;used[index]=true;
            }}
            for (index,_) in used.into_iter().enumerate().filter(|(_,used)|*used){
                let _=write!(sequence,"#{index}");
                let row=&planes[index*side..(index+1)*side];let mut x=0;
                while x<side {
                    let bits=row[x];let count=row[x..].iter().take_while(|&&v|v==bits).count();
                    if count>3 {let _=write!(sequence,"!{count}{}",char::from(bits+63));}else{for _ in 0..count{sequence.push(char::from(bits+63));}}x+=count;
                }
                sequence.push('$');
            }
            if y+6<side{sequence.push('-');}
        }
        sequence.push_str("\x1b\\\x1b8");
        Some(Self{rect,size:(size.columns,size.rows,size.width,size.height),background,sequence})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foot_detection_respects_overrides_and_multiplexers() {
        let env=|pairs: &[(&str,&str)]| pairs.iter().map(|(k,v)|(k.to_string(),v.to_string())).collect::<std::collections::HashMap<_,_>>();
        for (pairs,parent,expected) in [
            (vec![("TERM","foot")],false,true),
            (vec![("TERM","foot-extra")],false,true),
            (vec![("TERM","xterm-256color")],true,true),
            (vec![("TERM","xterm-256color")],false,false),
            (vec![("TERM","foot"),("TMUX","session")],true,false),
            (vec![("TERM","foot"),("LENZ_IMAGES","cells")],true,false),
            (vec![("TERM","foot"),("LENZ_IMAGES","kitty")],true,false),
            (vec![("LENZ_IMAGES","sixel")],false,true),
        ] {
            let vars=env(&pairs);
            assert_eq!(supported(|name|vars.get(name).cloned(),||parent),expected);
        }
        assert!(!supported(|_|Some("cells".into()),||panic!("no launcher lookup for an explicit override")));
        assert!(!crate::md_images::kitty_supported(|name|match name{"LENZ_IMAGES"=>Some("sixel".into()),"TERM_PROGRAM"=>Some("ghostty".into()),_=>None}),"the Sixel override must supersede Kitty auto-detection");
    }
    #[test]
    fn sixel_fits_known_cell_pixels_and_preserves_the_logo() {
        let rect=ratatui::layout::Rect::new(12,7,24,12);
        let size=crossterm::terminal::WindowSize{columns:100,rows:30,width:900,height:540};
        let image=Logo::new(rect,&size,0x20242a).unwrap();
        assert!(image.sequence.starts_with("\x1b7\x1b[8;13H\x1bP0;1;0q\"1;1;216;216"));
        assert!(image.sequence.ends_with("\x1b\\\x1b8"));
        assert!(image.sequence.len()<200_000);
        let decoded=decode(&image.sequence,216);
        let asset=include_bytes!("../../../assets/lenz-about-sixel.bin");
        for (x,y) in [(100,100),(50,50),(150,120),(210,210)] {
            let index=usize::from(asset[1024+(y*96/216)*96+x*96/216]);
            let p=&asset[index*4..index*4+4];
            let alpha=u32::from(p[3]);
            let expected=[16,8,0].map(|shift| {
                let channel=match shift{16=>p[0],8=>p[1],_=>p[2]};
                let blended=(u32::from(channel)*alpha+((0x20242a>>shift)&255)*(255-alpha)+127)/255;
                ((blended*100+127)/255*255+50)/100
            });
            assert_eq!(decoded[y*216+x],expected.map(|v|v as u8));
        }
        assert!(Logo::new(rect,&crossterm::terminal::WindowSize{width:0,..size},0).is_none());
        assert!(Logo::new(ratatui::layout::Rect::default(),&size,0).is_none());
    }
    fn decode(sequence:&str,side:usize)->Vec<[u8;3]> {
        let body=sequence.split('q').nth(1).unwrap().split("\x1b\\").next().unwrap().as_bytes();
        let mut palette=[[0;3];256];let mut pixels=vec![[0;3];side*side];
        let(mut i,mut x,mut y,mut color)=(0,0,0,0);
        while i<body.len(){
            let token=body[i];i+=1;
            let number=|i:&mut usize|{let start=*i;while *i<body.len()&&body[*i].is_ascii_digit(){*i+=1;}std::str::from_utf8(&body[start..*i]).unwrap().parse::<usize>().unwrap()};
            match token {
                b'"'=>{for n in 0..4{number(&mut i);if n<3{assert_eq!(body[i],b';');i+=1;}}},
                b'#'=>{color=number(&mut i);if body.get(i)==Some(&b';'){i+=1;assert_eq!(number(&mut i),2);for c in &mut palette[color]{assert_eq!(body[i],b';');i+=1;*c=((number(&mut i)*255+50)/100)as u8;}}},
                b'$'=>x=0,b'-'=>{x=0;y+=6;},
                b'!'|b'?'..=b'~'=>{let (count,bits)=if token==b'!'{let count=number(&mut i);let bits=body[i]-63;i+=1;(count,bits)}else{(1,token-63)};for _ in 0..count{for bit in 0..6{if bits&(1<<bit)!=0&&y+bit<side{pixels[(y+bit)*side+x]=palette[color];}}x+=1;}},
                _=>panic!("unexpected Sixel byte {token}"),
            }
        }
        pixels
    }
}
