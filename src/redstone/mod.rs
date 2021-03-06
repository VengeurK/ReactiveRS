extern crate std;
extern crate reactive_rs;
extern crate piston;
extern crate graphics;
extern crate glutin_window;
extern crate opengl_graphics;

use self::piston::window::WindowSettings;
use self::piston::event_loop::*;
use self::piston::input::*;
use self::glutin_window::GlutinWindow as Window;
use self::opengl_graphics::{ GlGraphics, OpenGL };

use reactive_rs::reactive::process::*;
use reactive_rs::reactive::signal::value_signal::*;

use std::ops::{Add, Sub, Mul};
use std::cmp::max;
use std::sync::{Arc, Mutex};
use std::thread;
use std::fs::File;
use std::io::prelude::*;

#[derive(PartialEq, Clone, Copy)]
enum Direction {
    SOUTH,
    NORTH,
    EAST,
    WEST
}

#[derive(Clone, Copy)]
enum Type {
    VOID,
    BLOCK,
    REDSTONE(Power),
    INVERTER(Direction),
    USER,
}

fn displace((x, y): (usize, usize), dir: Direction) -> (usize, usize){
    match dir {
        Direction::SOUTH => return (x  , y+1),
        Direction::NORTH => return (x  , y-1),
        Direction::EAST  => return (x+1, y  ),
        Direction::WEST  => return (x-1, y  ),
    }
}

fn invert_dir(dir: Direction) -> Direction {
    match dir {
        Direction::SOUTH => return Direction::NORTH,
        Direction::NORTH => return Direction::SOUTH,
        Direction::EAST  => return Direction::WEST,
        Direction::WEST  => return Direction::EAST,
    }
}

#[derive(PartialEq, Clone, Copy)]
struct Power {
    r: u8,
    g: u8,
    b: u8,
}

impl Add for Power {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        return Power{
            r: self.r + other.r,
            g: self.g + other.g,
            b: self.b + other.b}
    }
}

impl Sub for Power {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        return Power{
            r: self.r - other.r,
            g: self.g - other.g,
            b: self.b - other.b}
    }
}

impl Mul for Power {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        return Power{
            r: self.r * other.r,
            g: self.g * other.g,
            b: self.b * other.b}
    }
}
//
//impl Power {
//    fn less_than(self, other: Power) -> bool {
//           self.r <= other.r
//        && self.g <= other.g
//        && self.b <= other.b
//        && (
//               self.r < other.r
//            || self.g < other.g
//            || self.b < other.b)
//    }
//}

fn max_p(p: Power, q: Power) -> Power {
    Power{
        r: max(p.r, q.r),
        g: max(p.g, q.g),
        b: max(p.b, q.b)}
}

const ZERO_POWER: Power = Power{r: 0x0, g: 0x0, b: 0x0};
const ATOMIC_POWER: Power = Power{r: 0x1, g: 0x1, b: 0x1};
const MAX_POWER: Power = Power{r: 0xF, g: 0xF, b: 0xF};

pub fn redstone_sim() {
    let (blocks, w, h) = read_file(String::from("map.txt"));

    let mut power_signal = Vec::new();
    for i in 0..(w*h) {
        let filter =
            match blocks[i] {
                Type::VOID => ZERO_POWER,
                Type::BLOCK => ATOMIC_POWER,
                Type::REDSTONE(filter) => filter,
                Type::INVERTER(_) => ATOMIC_POWER,
                Type::USER => ATOMIC_POWER,
            };
        power_signal.push(ValueSignal::new(ZERO_POWER, Box::new(move |x: Power, y: Power| {
            max_p(x, y) * filter
        })));
    }
    let display_signal = ValueSignal::new(vec!(), Box::new(|entries: Vec<(usize, usize, Power)>, entry: (usize, usize, Power)| {
        let mut entries = entries.clone();
        entries.push(entry);
        entries
    }));
    let power_at = |(x, y): (usize, usize)| power_signal[(x % w) + (y % h) * w].clone();

    let redstone_wire_process = |x: usize, y: usize, filter: Power| {

        let decr = move|p: Power| {
            max_p(p, ATOMIC_POWER) - ATOMIC_POWER
        };
        let continue_loop: LoopStatus<()> = LoopStatus::Continue;
        let input = power_at((x, y));
        let combine_with_pos = move|power| (x, y, power * filter);
        let uncombine = move|(_x, _y, power)| power;
        input.emit(
            power_at((x + 1, y    )).emit(
                power_at((x - 1, y    )).emit(
                    power_at((x    , y + 1)).emit(
                        power_at((x    , y - 1)).emit(
                            display_signal.emit(
                                input.await().map(combine_with_pos)).map(uncombine).map(decr))))))
            .then(value(continue_loop)).while_loop()
    };

    let blocks_copy = blocks.clone();
    let redstone_torch_process = |x: usize, y: usize, dir: Direction| {
        let input = power_at(displace((x, y), invert_dir(dir)));
        let is_powered = |power| {
            power != ZERO_POWER
        };
        let should_emit = |pos| {
            let (x, y) = pos;
            match blocks_copy[x+w*y] {
                Type::REDSTONE(_) => true,
                Type::BLOCK => true,
                _ => false
            }
        };
        let mut emit_near = vec!(power_at((x, y)).emit(value(MAX_POWER)));
        for d in vec!(Direction::NORTH, Direction::SOUTH, Direction::EAST, Direction::WEST) {
            if d != invert_dir(dir) && should_emit(displace((x, y), d)) {
                emit_near.push(power_at(displace((x, y), d)).emit(value(MAX_POWER)))
            }
        }
        let continue_loop: LoopStatus<()> = LoopStatus::Continue;
        let p = input.emit(value(ZERO_POWER)).then(if_else(input.await().map(is_powered), value(()), multi_join(emit_near).then(display_signal.emit(value((x, y, MAX_POWER)))).then(value(()))));
        p.then(value(continue_loop)).while_loop()
    };

    let user_press = Arc::new(Mutex::new(false));
    let redstone_user_process = |x: usize, y: usize| {
        let mut emit_near = vec!();
        for d in vec!(Direction::NORTH, Direction::SOUTH, Direction::EAST, Direction::WEST) {
            emit_near.push(power_at(displace((x, y), d)).emit(value(MAX_POWER)))
        }
        let continue_loop: LoopStatus<()> = LoopStatus::Continue;
        let user_press = user_press.clone();
        let is_user_active = move|()| {
            *user_press.lock().unwrap()
        };
        let p = if_else(value(()).map(is_user_active).pause(), value(()), multi_join(emit_near).then(display_signal.emit(value((x, y, MAX_POWER)))).then(value(())));
        p.then(value(continue_loop)).while_loop()
    };

    let display_powers: Arc<Mutex<Vec<Power>>> = Arc::new(Mutex::new(vec![ZERO_POWER; w*h]));
    let display_powers_ref = display_powers.clone();

    let display_process = || {
        let mut powers = Vec::new();
        for _ in 0..(w*h) {
            powers.push(ZERO_POWER);
        }
        let powers: Arc<Mutex<Vec<Power>>> = Arc::new(Mutex::new(powers));
        let continue_loop: LoopStatus<()> = LoopStatus::Continue;
        let powers_ref = powers.clone();
        let read_entries = move|entries: Vec<(usize, usize, Power)>| {
            let mut powers = powers_ref.lock().unwrap();
            for i in 0..(w*h) {
                (*powers)[i] = ZERO_POWER;
            }
            for (x, y, power) in entries {
                (*powers)[x + y * w] = power;
            }
        };
        let powers_ref = powers.clone();
        let draw = move|_| {
//            use std::thread;
//            use std::time::Duration;
//            thread::sleep(Duration::from_millis(150));
            let mut dpowers = display_powers_ref.lock().unwrap();
            let powers = powers_ref.lock().unwrap();
            dpowers.clone_from(&powers);
        };
        display_signal.await().map(read_entries).map(draw).then(value(continue_loop)).while_loop()
    };

    let mut p_redstone = Vec::new();
    let mut p_inverter = Vec::new();
    let mut p_user = Vec::new();
    for x in 0..w {
        for y in 0..h {
            match blocks[x + y * w] {
                Type::VOID => (),
                Type::BLOCK => (),
                Type::REDSTONE(filter) => p_redstone.push(redstone_wire_process(x, y, filter)),
                Type::INVERTER(dir) => p_inverter.push(redstone_torch_process(x, y, dir)),
                Type::USER => p_user.push(redstone_user_process(x, y)),
            }
        }
    }

    let display_powers_ref = display_powers.clone();
    let user_press = user_press.clone();
    thread::spawn(move || {
        //let opengl = OpenGL::V2_1;
        let opengl = OpenGL::V3_2;

        let mut window: Window = WindowSettings::new(
            "redstone",
            [1280, 720]
        )
            .opengl(opengl)
            .exit_on_esc(true)
            .srgb(false) // Necessary due to issue #139 of piston_window.
            .build()
            .unwrap();

        let zoom_step: f64 = f64::powf(2.0, 1.0/7.0);
        const ZOOM_INIT: f64 = 10.0;

        let mut app = App {
            gl: GlGraphics::new(opengl),
            powers: vec![ZERO_POWER; blocks.len()],
            blocks: blocks,
            width: w,
            height: h,
            zoom: ZOOM_INIT,
            tx: 0.0,
            ty: 0.0
        };


        let mut events = Events::new(EventSettings::new());
        while let Some(e) = events.next(&mut window) {
            if let Some(r) = e.render_args() {
                {
                    let mut dpowers = display_powers_ref.lock().unwrap();
                    app.powers.clone_from(&dpowers)
                }
                app.render(&r);
            }
            if Some(Button::Keyboard(Key::Backspace)) == e.press_args(){
                app.zoom *= zoom_step;
                app.tx *= zoom_step;
                app.ty *= zoom_step;
            }
            if Some(Button::Keyboard(Key::Return)) == e.press_args(){
                app.zoom /= zoom_step;
                app.tx == zoom_step;
                app.ty == zoom_step;
            }
            if Some(Button::Keyboard(Key::Left)) == e.press_args(){
                app.tx += app.zoom;
            }
            if Some(Button::Keyboard(Key::Right)) == e.press_args(){
                app.tx -= app.zoom;
            }
            if Some(Button::Keyboard(Key::Up)) == e.press_args(){
                app.ty += app.zoom;
            }
            if Some(Button::Keyboard(Key::Down)) == e.press_args(){
                app.ty -= app.zoom;
            }
            if Some(Button::Keyboard(Key::Space)) == e.press_args(){
                *user_press.lock().unwrap() = true;
            }
            if Some(Button::Keyboard(Key::Space)) == e.release_args() {
                *user_press.lock().unwrap() = false;
            }
        }
    });

    execute_process(multi_join(p_redstone).join(multi_join(p_inverter)).join(multi_join(p_user)).join(display_process()));

}

fn read_file(filename: String) -> (Vec<Type>, usize, usize) {
    let mut file = File::open(filename).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();

    let mut blocks: Vec<Type> = Vec::new();
    let mut width = 0;
    let mut height = 0;

    let mut lines = contents.lines();
    while let Some(line) = lines.next() {
        if height == 0 {
            width = line.len();
        } else {
            assert_eq!(width, line.len());
        }
        height += 1;
        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            blocks.push(match ch {
                '.' => Type::VOID,
                '#' => Type::BLOCK,
                '@' => Type::USER,
                'r' => Type::REDSTONE(Power{r: 0x1, g: 0x0, b: 0x0}),
                'g' => Type::REDSTONE(Power{r: 0x0, g: 0x1, b: 0x0}),
                'b' => Type::REDSTONE(Power{r: 0x0, g: 0x0, b: 0x1}),
                'y' => Type::REDSTONE(Power{r: 0x1, g: 0x1, b: 0x0}),
                'p' => Type::REDSTONE(Power{r: 0x1, g: 0x0, b: 0x1}),
                'c' => Type::REDSTONE(Power{r: 0x0, g: 0x1, b: 0x1}),
                'w' => Type::REDSTONE(Power{r: 0x1, g: 0x1, b: 0x1}),
                '^' => Type::INVERTER(Direction::NORTH),
                'v' => Type::INVERTER(Direction::SOUTH),
                '<' => Type::INVERTER(Direction::WEST),
                '>' => Type::INVERTER(Direction::EAST),
                _ => panic!("Not a valid character")
            });
        }
    }

    (blocks, width, height)
}

pub struct App {
    gl: GlGraphics, // OpenGL drawing backend.
    powers: Vec<Power>,
    blocks: Vec<Type>,
    width: usize,
    height: usize,
    zoom: f64,
    tx: f64,
    ty: f64
}

impl App {
    fn render(&mut self, args: &RenderArgs) {
        use self::graphics::*;

        const VOID_COLOR:       [f32; 4] = [0.0, 0.0, 0.0, 1.0];
        const BLOCK_COLOR_OUT:  [f32; 4] = [0.9, 0.9, 0.9, 1.0];
        const BLOCK_COLOR_IN:   [f32; 4] = [0.5, 0.5, 0.5, 1.0];
        const BORDER_SIZE: f64 = 2.0;
        const POWER_MAX:   u8  = 15;

        self.gl.draw(args.viewport(), |_c, gl| {
            clear(VOID_COLOR, gl);
        });

        let pixel_size = self.zoom;

        let square = rectangle::square(0.0, 0.0, pixel_size);
        let inner_square = rectangle::square(0.0, 0.0, pixel_size-2.0*BORDER_SIZE);
        let rect = rectangle::rectangle_by_corners(0.0, 0.0, pixel_size, pixel_size/3.0);

        for i in 0..(self.width*self.height) {
            let (ix, iy) = (i%self.width, i/self.width);
            let (x, y) = ((ix as f64)*pixel_size+self.tx, (iy as f64)*pixel_size+self.ty);

            fn color_composant(is_present: bool, power: u8) -> f32 {
                if is_present { 0.5 + 0.5*((power as f32)/(POWER_MAX as f32)) } else { 0.0 }
            }
            fn get_color(r: u8, g: u8, b: u8, power: Power) -> [f32; 4] {
                [
                    color_composant(r > 0, power.r),
                    color_composant(g > 0, power.g),
                    color_composant(b > 0, power.b),
                    1.0
                ]
            }

            match self.blocks[i] {
                Type::VOID => {
                    self.gl.draw(args.viewport(), |c, gl| {
                        let transform = c.transform.trans(x, y);
                        rectangle(VOID_COLOR, square, transform, gl);
                    });
                },
                Type::BLOCK => {
                    self.gl.draw(args.viewport(), |c, gl| {
                        let transform = c.transform.trans(x, y);
                        rectangle(BLOCK_COLOR_OUT, square, transform, gl);
                        let transform = c.transform.trans(x+BORDER_SIZE, y+BORDER_SIZE);
                        rectangle(BLOCK_COLOR_IN, inner_square, transform, gl);
                    });
                },
                Type::REDSTONE(Power{r, g, b}) => {
                    let color = get_color(r, g, b, self.powers[i]);
                    self.gl.draw(args.viewport(), |c, gl| {
                        let transform = c.transform.trans(x, y);
                        rectangle(color, square, transform, gl);
                    });
                },
                Type::INVERTER(ref dir) => {
                    let color = get_color(1, 1, 1, self.powers[i]);
                    self.gl.draw(args.viewport(), |c, gl| {
                        let pi = std::f64::consts::PI;
                        let angle = pi/2.0 * match *dir {
                            Direction::SOUTH => 0.0,
                            Direction::NORTH => 2.0,
                            Direction::EAST => 3.0,
                            Direction::WEST => 1.0
                        };
                        let transform = c.transform.trans(x, y).trans(pixel_size/2.0, pixel_size/2.0).rot_rad(angle).trans(-pixel_size/2.0, -pixel_size/2.0);
                        let transform2 = transform.rot_rad(pi/2.0).trans(0.0, -pixel_size*(0.5+1.0/6.0));
                        rectangle(color, rect, transform, gl);
                        rectangle(color, rect, transform2, gl);
                    });
                },
                Type::USER => {
                    self.gl.draw(args.viewport(), |c, gl| {
                        let transform = c.transform.trans(x, y);
                        rectangle(BLOCK_COLOR_IN, square, transform, gl);
                        let transform = c.transform.trans(x+BORDER_SIZE, y+BORDER_SIZE);
                        rectangle(BLOCK_COLOR_OUT, inner_square, transform, gl);
                    });
                }
            }
        }
    }
}
