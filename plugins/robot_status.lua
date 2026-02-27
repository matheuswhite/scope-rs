local status = {
    a = 0,
    b = 0,
    c = 0,
    d = 0,
    quaternion = { w = 0, x = 0, y = 0, z = 0 },
    xyz = { x = 0, y = 0, z = 0 },
    pose = { w = 0, x = 0, y = 0, z = 0 },
}
status.__index = status

local function decode_float(float, pos)
    local f = float[pos] | (float[pos + 1] << 8) | (float[pos + 2] << 16) | (float[pos + 3] << 24)

    local sign = (f >> 31) & 0x1
    local exponent = (f >> 23) & 0xFF
    local mantissa = f & 0x7FFFFF

    if exponent == 0 then
        return (-1) ^ sign * (mantissa / (2 ^ 23)) * (2 ^ -126)
    elseif exponent == 255 then
        return mantissa == 0 and ((-1) ^ sign * math.huge) or (0 / 0)
    else
        return (-1) ^ sign * (1 + mantissa / (2 ^ 23)) * (2 ^ (exponent - 127))
    end
end

local function decode_quaternion(bytes, pos)
    return {
        w = decode_float(bytes, pos),
        x = decode_float(bytes, pos + 4),
        y = decode_float(bytes, pos + 8),
        z = decode_float(bytes, pos + 12),
    }
end

local function decode_xyz(bytes, pos)
    return {
        x = decode_float(bytes, pos),
        y = decode_float(bytes, pos + 4),
        z = decode_float(bytes, pos + 8),
    }
end

function status.decode(bytes)
    local self = setmetatable({}, status)

    self.a = bytes[1]
    self.b = bytes[2]
    self.c = bytes[3]
    self.d = bytes[4]
    self.quaternion = decode_quaternion(bytes, 5)
    self.xyz = decode_xyz(bytes, 21)
    self.pose = decode_quaternion(bytes, 33)

    return self
end

function status.size()
    return 48
end

function status:__tostring()
    return string.format(
        "a: %d, b: %d, c: %d, d: %d, quaternion: {w: %.2f, x: %.2f, y: %.2f, z: %.2f}, xyz: {x: %.2f, y: %.2f, z: %.2f}, pose: {w: %.2f, x: %.2f, y: %.2f, z: %.2f}",
        self.a, self.b, self.c, self.d,
        self.quaternion.w, self.quaternion.x, self.quaternion.y, self.quaternion.z,
        self.xyz.x, self.xyz.y, self.xyz.z,
        self.pose.w, self.pose.x, self.pose.y, self.pose.z)
end

return status
