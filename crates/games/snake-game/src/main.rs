use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::Rng;
use std::{
    io::{self, Stdout, Write},
    time::{Duration, Instant},
};

// Game constants
const WIDTH: u16 = 80;
const HEIGHT: u16 = 24;
const INITIAL_SPEED: Duration = Duration::from_millis(150);
const SPEED_INCREMENT: Duration = Duration::from_millis(5);

// Game types
#[derive(Clone, Copy, Debug, PartialEq)]
struct Position {
    x: u16,
    y: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

struct Snake {
    body: Vec<Position>,
    direction: Direction,
}

impl Snake {
    fn new() -> Self {
        let mut body = Vec::new();
        // Start with 3 segments in the middle
        body.push(Position { x: WIDTH / 2, y: HEIGHT / 2 });
        body.push(Position { x: WIDTH / 2 - 1, y: HEIGHT / 2 });
        body.push(Position { x: WIDTH / 2 - 2, y: HEIGHT / 2 });
        
        Self {
            body,
            direction: Direction::Right,
        }
    }
    
    fn move_forward(&mut self) {
        let head = self.body[0];
        let new_head = match self.direction {
            Direction::Up => Position { x: head.x, y: if head.y == 0 { HEIGHT - 1 } else { head.y - 1 } },
            Direction::Down => Position { x: head.x, y: if head.y == HEIGHT - 1 { 0 } else { head.y + 1 } },
            Direction::Left => Position { x: if head.x == 0 { WIDTH - 1 } else { head.x - 1 }, y: head.y },
            Direction::Right => Position { x: if head.x == WIDTH - 1 { 0 } else { head.x + 1 }, y: head.y },
        };
        
        // Move body: add new head, remove tail
        self.body.insert(0, new_head);
        self.body.pop();
    }
    
    fn grow(&mut self) {
        let head = self.body[0];
        self.body.insert(0, head);
    }
    
    fn change_direction(&mut self, new_direction: Direction) {
        // Prevent 180-degree turns
        match (self.direction, new_direction) {
            (Direction::Up, Direction::Down) | 
            (Direction::Down, Direction::Up) | 
            (Direction::Left, Direction::Right) | 
            (Direction::Right, Direction::Left) => {}
            _ => self.direction = new_direction,
        }
    }
    
    fn is_colliding_with_self(&self) -> bool {
        let head = self.body[0];
        self.body[1..].iter().any(|pos| *pos == head)
    }
}

struct Food {
    position: Position,
}

impl Food {
    fn new(snake: &Snake) -> Self {
        let mut rng = rand::rng();
        let mut position;
        
        loop {
            position = Position {
                x: rng.gen_range(0..WIDTH),
                y: rng.gen_range(0..HEIGHT),
            };
            
            // Make sure food doesn't appear on snake
            if !snake.body.iter().any(|p| *p == position) {
                break;
            }
        }
        
        Self { position }
    }
    
    fn respawn(&mut self, snake: &Snake) {
        let mut rng = rand::rng();
        
        loop {
            self.position = Position {
                x: rng.gen_range(0..WIDTH),
                y: rng.gen_range(0..HEIGHT),
            };
            
            // Make sure food doesn't appear on snake
            if !snake.body.iter().any(|p| *p == self.position) {
                break;
            }
        }
    }
}

struct Game {
    snake: Snake,
    food: Food,
    score: u32,
    speed: Duration,
    game_over: bool,
}

impl Game {
    fn new() -> Self {
        let snake = Snake::new();
        let food = Food::new(&snake);
        
        Self {
            snake,
            food,
            score: 0,
            speed: INITIAL_SPEED,
            game_over: false,
        }
    }
    
    fn update(&mut self) {
        if self.game_over {
            return;
        }
        
        self.snake.move_forward();
        
        // Check collision with food
        if self.snake.body[0] == self.food.position {
            self.snake.grow();
            self.food.respawn(&self.snake);
            self.score += 10;
            
            // Increase speed every 50 points
            if self.score % 50 == 0 && self.speed > Duration::from_millis(50) {
                self.speed -= SPEED_INCREMENT;
            }
        }
        
        // Check collision with self
        if self.snake.is_colliding_with_self() {
            self.game_over = true;
        }
    }
    
    fn handle_input(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Up => self.snake.change_direction(Direction::Up),
            KeyCode::Down => self.snake.change_direction(Direction::Down),
            KeyCode::Left => self.snake.change_direction(Direction::Left),
            KeyCode::Right => self.snake.change_direction(Direction::Right),
            KeyCode::Char('q') | KeyCode::Char('Q') => self.game_over = true,
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if self.game_over {
                    *self = Self::new();
                }
            }
            _ => {}
        }
    }
}

fn draw_frame(stdout: &mut Stdout, game: &Game) -> io::Result<()> {
    // Clear screen
    execute!(stdout, terminal::Clear(terminal::ClearType::All))?;
    
    // Draw border
    for x in 0..WIDTH {
        execute!(stdout, crossterm::cursor::MoveTo(x, 0))?;
        write!(stdout, "─")?;
        
        execute!(stdout, crossterm::cursor::MoveTo(x, HEIGHT - 1))?;
        write!(stdout, "─")?;
    }
    
    for y in 0..HEIGHT {
        execute!(stdout, crossterm::cursor::MoveTo(0, y))?;
        write!(stdout, "│")?;
        
        execute!(stdout, crossterm::cursor::MoveTo(WIDTH - 1, y))?;
        write!(stdout, "│")?;
    }
    
    // Draw corners
    execute!(stdout, crossterm::cursor::MoveTo(0, 0))?;
    write!(stdout, "┌")?;
    
    execute!(stdout, crossterm::cursor::MoveTo(WIDTH - 1, 0))?;
    write!(stdout, "┐")?;
    
    execute!(stdout, crossterm::cursor::MoveTo(0, HEIGHT - 1))?;
    write!(stdout, "└")?;
    
    execute!(stdout, crossterm::cursor::MoveTo(WIDTH - 1, HEIGHT - 1))?;
    write!(stdout, "┘")?;
    
    // Draw snake
    for (i, pos) in game.snake.body.iter().enumerate() {
        execute!(stdout, crossterm::cursor::MoveTo(pos.x, pos.y))?;
        if i == 0 {
            // Head
            write!(stdout, "●")?;
        } else {
            // Body
            write!(stdout, "○")?;
        }
    }
    
    // Draw food
    execute!(stdout, crossterm::cursor::MoveTo(game.food.position.x, game.food.position.y))?;
    write!(stdout, "🍎")?;
    
    // Draw score
    execute!(stdout, crossterm::cursor::MoveTo(2, HEIGHT - 1))?;
    write!(stdout, "Score: {}", game.score)?;
    
    // Draw game over message
    if game.game_over {
        execute!(stdout, crossterm::cursor::MoveTo(WIDTH / 2 - 7, HEIGHT / 2))?;
        write!(stdout, "GAME OVER!")?;
        execute!(stdout, crossterm::cursor::MoveTo(WIDTH / 2 - 10, HEIGHT / 2 + 1))?;
        write!(stdout, "Press R to restart")?;
        execute!(stdout, crossterm::cursor::MoveTo(WIDTH / 2 - 8, HEIGHT / 2 + 2))?;
        write!(stdout, "Press Q to quit")?;
    }
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    
    // Setup game
    let mut game = Game::new();
    let mut last_update = Instant::now();
    
    // Main game loop
    loop {
        // Handle input
        if let Event::Key(key_event) = event::read()? {
            game.handle_input(key_event);
        }
        
        // Update game state
        if last_update.elapsed() >= game.speed {
            game.update();
            last_update = Instant::now();
        }
        
        // Draw frame
        draw_frame(&mut stdout, &game)?;
        
        // Check for exit condition
        if game.game_over {
            // Wait for key press to exit or restart
            loop {
                if let Event::Key(key_event) = event::read()? {
                    match key_event.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            return Ok(());
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            game = Game::new();
                            break;
                        }
                        _ => continue,
                    }
                }
            }
        }
        
        // Small delay to prevent excessive CPU usage
        std::thread::sleep(Duration::from_millis(10));
    }
    
    // Cleanup terminal
    execute!(stdout, LeaveAlternateScreen, Show)?;
    
    Ok(())
}
