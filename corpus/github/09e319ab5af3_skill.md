# Weather MCP Skills

This MCP server provides the following skills (tools) for interacting with weather data.

## Tools

### get_weather

**Description**: Retrieves the current weather for a specified city.

**Parameters**:
- `city` (string): The name of the city to get weather for (e.g., "Beijing", "New York").

**Returns**: A string describing the current weather, including temperature and conditions.

**Example Usage**:
- Input: `{"city": "Beijing"}`
- Output: `"beijing: ⛅ +14C"`

**Notes**:
- Uses the wttr.in API for weather data.
- Returns data in a simple text format.
- If the city is not found or API fails, returns an error message.

## Integration

This skill can be integrated with MCP-compatible clients like Claude Desktop for seamless weather queries in conversations.
